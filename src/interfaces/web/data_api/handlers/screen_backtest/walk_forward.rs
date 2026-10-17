//! `/stocks/screen/backtest/walk-forward` handler：滾動前推最佳化。
//!
//! 偏移穩健度、前緣與樣本內外分割都還是「看過整段資料之後」挑參數；這裡模擬當時真的會怎麼做：
//! 每年年初只用之前 `train_years` 年的期數，在 [`ETF_WEIGHTS`] × [`MAX_WEIGHTS`] × 換股緩衝
//! （`limit`、1.5 倍、2 倍）的組合中挑扣成本後夏普值最高的一組，套用到這一年；把各年的樣本外期數
//! 串起來彙總，並與「不調參數」（請求本身的 `etf_weight`、`max_weight`、`keep_rank`）比較。
//!
//! 每個換股日偏移只查一次候選股（讀到最大的緩衝名次），各組合只在記憶體中重算持股、權重、成本與
//! 0050 混合；各組合的成本是自己整段連續持有算出來的，換參數當下的調整成本不另計。偏移的取法與
//! 偏移穩健度相同，回傳各偏移結果的分布，另列 `rebalance_offset` 那一個偏移的逐年明細。

use std::ops::Range;

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Datelike, NaiveDate};
use futures::{StreamExt, TryStreamExt, stream};

use super::offsets::{CONCURRENCY, distribution, sample_offsets};
use super::portfolio::PeriodCost;
use super::preset::BacktestQuery;
use super::summary::{compound, summarize};
use super::{Backtest, ETF_BENCHMARK, MAX_KEEP_RANK, PeriodRow, Simulation, round2};
use crate::interfaces::web::data_api::dto::{
    BacktestOffsetResult, BacktestSummary, BacktestWalkForwardYear, ErrorBody,
    ScreenBacktestParams, ScreenBacktestWalkForwardResponse,
};
use crate::interfaces::web::data_api::handlers::{database_error, error_response};

/// 候選的 0050 核心部位（%）。
pub(super) const ETF_WEIGHTS: [u8; 4] = [0, 30, 50, 70];

/// 候選的單檔權重上限（%）；`None` 為不設上限。
pub(super) const MAX_WEIGHTS: [Option<u8>; 3] = [None, Some(10), Some(20)];

/// 預設訓練年數。
const DEFAULT_TRAIN_YEARS: u8 = 3;

/// 一組候選參數。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Candidate {
    pub(super) etf_weight: u8,
    pub(super) max_weight: Option<u8>,
    pub(super) keep_rank: u8,
}

/// 候選的換股緩衝：`limit`、1.5 倍、2 倍（不超過 [`MAX_KEEP_RANK`]，去除重複）。
pub(super) fn keep_ranks(limit: u8) -> Vec<u8> {
    let mut ranks: Vec<u8> = Vec::new();
    for rank in [
        u16::from(limit),
        u16::from(limit) * 3 / 2,
        u16::from(limit) * 2,
    ] {
        let rank = rank.min(u16::from(MAX_KEEP_RANK)) as u8;
        if !ranks.contains(&rank) {
            ranks.push(rank);
        }
    }
    ranks
}

/// 全部候選組合，依緩衝、上限、0050 比例的順序；同分時取先出現的（緩衝與 0050 比例較低）。
pub(super) fn candidates(limit: u8) -> Vec<Candidate> {
    keep_ranks(limit)
        .into_iter()
        .flat_map(|keep_rank| {
            MAX_WEIGHTS.into_iter().flat_map(move |max_weight| {
                ETF_WEIGHTS.into_iter().map(move |etf_weight| Candidate {
                    etf_weight,
                    max_weight,
                    keep_rank,
                })
            })
        })
        .collect()
}

/// 一組參數整段的各期結果（已移除還沒滿持有天數的最後一期）。
#[derive(Debug, Clone, Default)]
pub(super) struct Track {
    pub(super) rows: Vec<PeriodRow>,
    pub(super) costs: Vec<PeriodCost>,
}

impl Track {
    /// 換股日落在 `[start, end)` 的期數範圍（`rows` 依換股日由舊到新）。
    fn range(&self, start: NaiveDate, end: NaiveDate) -> Range<usize> {
        let from = self.rows.partition_point(|row| row.date < start);
        let to = self.rows.partition_point(|row| row.date < end);
        from..to
    }

    /// `range` 期數的扣成本後夏普值；不足 2 期或沒有波動為 `None`。
    fn sharpe(&self, range: Range<usize>, hold_days: u16) -> Option<f64> {
        summarize(&self.rows[range.clone()], &self.costs[range], hold_days).sharpe_ratio
    }
}

/// 一個偏移的滾動前推結果：每年挑出的參數與明細，以及串起來的樣本外期數（滾動與不調參數）。
#[derive(Debug, Clone, Default)]
pub(super) struct WalkForward {
    pub(super) years: Vec<BacktestWalkForwardYear>,
    pub(super) chosen: Track,
    pub(super) baseline: Track,
}

/// 年初。
fn year_start(year: i32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, 1, 1).unwrap_or_default()
}

/// 一段期數的扣成本後累積報酬與 0050 累積報酬（%）；0050 全缺為 `None`。
fn period_returns(track: &Track, range: Range<usize>) -> (f64, Option<f64>) {
    let (net, _) = compound(
        track.costs[range.clone()]
            .iter()
            .map(|cost| cost.net_return),
    );
    let rows = &track.rows[range];
    let etf = rows
        .iter()
        .any(|row| row.etf_return.is_some())
        .then(|| (compound(rows.iter().map(|row| row.etf_return.unwrap_or(0.0))).0 - 1.0) * 100.0);
    ((net - 1.0) * 100.0, etf)
}

/// 依 `tracks`（與 [`candidates`] 一一對應）與不調參數的 `baseline` 做滾動前推：第一個樣本外年度為
/// 起日所在年度加 `train_years`；訓練期間每組參數的夏普值都沒有（期數不足）的年份略過。
pub(super) fn walk_forward(
    candidates: &[Candidate],
    tracks: &[Track],
    baseline: &Track,
    from: NaiveDate,
    to: NaiveDate,
    train_years: u8,
    hold_days: u16,
) -> WalkForward {
    let mut result = WalkForward::default();
    for year in from.year() + i32::from(train_years)..=to.year() {
        let (train_start, test_start, test_end) = (
            year_start(year - i32::from(train_years)),
            year_start(year),
            year_start(year + 1),
        );
        let best = candidates
            .iter()
            .zip(tracks)
            .filter_map(|(candidate, track)| {
                let sharpe = track.sharpe(track.range(train_start, test_start), hold_days)?;
                Some((candidate, track, sharpe))
            })
            .fold(
                None,
                |best: Option<(&Candidate, &Track, f64)>, item| match best {
                    Some(top) if top.2 >= item.2 => Some(top),
                    _ => Some(item),
                },
            );
        let Some((candidate, track, sharpe)) = best else {
            continue;
        };
        let test = track.range(test_start, test_end);
        if test.is_empty() {
            continue;
        }
        let base = baseline.range(test_start, test_end);
        let (return_after_costs, etf_return) = period_returns(track, test.clone());
        let (baseline_return, _) = period_returns(baseline, base.clone());
        result.years.push(BacktestWalkForwardYear {
            year,
            etf_weight: candidate.etf_weight,
            max_weight: candidate.max_weight,
            keep_rank: candidate.keep_rank,
            train_sharpe_ratio: round2(sharpe),
            periods: test.len() as i64,
            return_after_costs: round2(return_after_costs),
            baseline_return_after_costs: round2(baseline_return),
            etf_return: etf_return.map(round2),
        });
        result
            .chosen
            .rows
            .extend_from_slice(&track.rows[test.clone()]);
        result.chosen.costs.extend_from_slice(&track.costs[test]);
        result
            .baseline
            .rows
            .extend_from_slice(&baseline.rows[base.clone()]);
        result
            .baseline
            .costs
            .extend_from_slice(&baseline.costs[base]);
    }
    result
}

impl Backtest<'_> {
    /// 一個偏移：查一次候選股，算出每組候選參數與不調參數的各期結果，再做滾動前推。
    async fn walk_forward(
        &self,
        offset: u16,
        candidates: &[Candidate],
        train_years: u8,
    ) -> Result<WalkForward, sqlx::Error> {
        let deepest = candidates
            .iter()
            .map(|candidate| candidate.keep_rank)
            .chain(std::iter::once(self.keep_rank))
            .max()
            .unwrap_or(self.keep_rank);
        let rows = self.candidates(offset, deepest).await?;
        let finish = |raw: super::Raw, etf_weight: u8| {
            let Simulation { rows, costs, .. } = raw.finish(etf_weight);
            Track { rows, costs }
        };
        let mut tracks: Vec<Track> = Vec::with_capacity(candidates.len());
        // 同一組緩衝與上限只組一次持股，各 0050 比例只重新混合。
        for group in
            candidates.chunk_by(|a, b| (a.keep_rank, a.max_weight) == (b.keep_rank, b.max_weight))
        {
            let raw = self
                .assemble(rows.clone(), group[0].keep_rank, group[0].max_weight)
                .await?;
            tracks.extend(
                group
                    .iter()
                    .map(|candidate| finish(raw.clone(), candidate.etf_weight)),
            );
        }
        let baseline = finish(
            self.assemble(rows, self.keep_rank, self.max_weight).await?,
            self.etf_weight,
        );
        Ok(walk_forward(
            candidates,
            &tracks,
            &baseline,
            self.from,
            self.to,
            train_years,
            self.hold_days,
        ))
    }
}

/// 滾動前推最佳化：每年用之前 `train_years` 年挑參數、套用到這一年，回傳逐年明細、串起來的彙總與
/// 各偏移的分布，並與不調參數比較。
///
/// 參數與 `/stocks/screen/backtest` 相同，另有 `train_years`（1–5，預設 3）；`etf_weight`、
/// `max_weight`、`keep_rank` 只用於「不調參數」的對照，`split` 不使用。
///
/// # Errors
///
/// 參數驗證與 `/stocks/screen/backtest` 相同（422），`train_years` 不在 1–5 或期間沒有超過訓練年數也
/// 回 422；迄日之前沒有相對強度資料回 404；驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/stocks/screen/backtest/walk-forward", tag = "data-api", params(ScreenBacktestParams), responses((status = 200, body = ScreenBacktestWalkForwardResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn screen_backtest_walk_forward(BacktestQuery(params): BacktestQuery) -> Response {
    let unprocessable = |message: &str| error_response(StatusCode::UNPROCESSABLE_ENTITY, message);
    let train_years = params.train_years.unwrap_or(DEFAULT_TRAIN_YEARS);
    if !(1..=5).contains(&train_years) {
        return unprocessable("train_years 必須介於 1 至 5");
    }
    let backtest = match Backtest::prepare(&params).await {
        Ok(backtest) => backtest,
        Err(response) => return *response,
    };
    if backtest.to.year() < backtest.from.year() + i32::from(train_years) {
        return unprocessable(
            "回測期間必須超過 train_years 年（樣本外年度從起日年度加訓練年數起）",
        );
    }
    let rebalance_offset = params.rebalance_offset.unwrap_or(0);
    if rebalance_offset >= backtest.hold_days {
        return unprocessable("rebalance_offset 必須介於 0 至 hold_days − 1");
    }
    let grid = candidates(backtest.limit);
    let mut offsets = sample_offsets(backtest.hold_days);
    if !offsets.contains(&rebalance_offset) {
        offsets.push(rebalance_offset);
    }
    let backtest = &backtest;
    let grid = &grid;
    let runs: Vec<(u16, WalkForward)> = match stream::iter(offsets)
        .map(|offset| async move {
            backtest
                .walk_forward(offset, grid, train_years)
                .await
                .map(|result| (offset, result))
        })
        .buffered(CONCURRENCY)
        .try_collect()
        .await
    {
        Ok(runs) => runs,
        Err(error) => return database_error(error),
    };
    let hold_days = backtest.hold_days;
    let summary_of = |track: &Track| summarize(&track.rows, &track.costs, hold_days);
    let results = |pick: fn(&WalkForward) -> &Track| -> Vec<BacktestOffsetResult> {
        runs.iter()
            .map(|(offset, run)| BacktestOffsetResult {
                rebalance_offset: *offset,
                summary: summary_of(pick(run)),
            })
            .collect()
    };
    let detail = runs
        .iter()
        .find(|(offset, _)| *offset == rebalance_offset)
        .map(|(_, run)| run.clone())
        .unwrap_or_default();
    let summary: BacktestSummary = summary_of(&detail.chosen);
    let baseline: BacktestSummary = summary_of(&detail.baseline);

    Json(ScreenBacktestWalkForwardResponse {
        from: backtest.from.to_string(),
        to: backtest.to.to_string(),
        market: backtest.market.to_owned(),
        hold_days,
        limit: backtest.limit,
        rank_by: backtest.rank_by.to_owned(),
        weighting: backtest.weighting.as_str().to_owned(),
        stop_loss: params.stop_loss,
        trailing_stop: params.trailing_stop,
        fee_discount: backtest.fee_discount,
        etf_benchmark: ETF_BENCHMARK.to_owned(),
        train_years,
        candidates: grid.len() as i64,
        rebalance_offset,
        offsets: distribution(&results(|run| &run.chosen)),
        baseline_offsets: distribution(&results(|run| &run.baseline)),
        years: detail.years,
        summary,
        baseline,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("有效日期")
    }

    /// 每季一期、報酬依序取 `returns`（扣成本後相同），0050 固定 1%。
    fn track(start: i32, returns: &[f64]) -> Track {
        let rows: Vec<PeriodRow> = returns
            .iter()
            .enumerate()
            .map(|(index, ret)| PeriodRow {
                date: day(start + index as i32 / 4, 1 + 3 * (index as u32 % 4), 2),
                holdings: 1,
                avg_return: Some(*ret),
                etf_return: Some(1.0),
                ..PeriodRow::default()
            })
            .collect();
        let costs = returns
            .iter()
            .map(|ret| PeriodCost {
                turnover: 0.0,
                net_return: *ret,
            })
            .collect();
        Track { rows, costs }
    }

    /// 緩衝依 limit 的 1、1.5、2 倍且不超過 200、去除重複；組合數為 3 × 3 × 4，順序先緩衝再上限再
    /// 0050 比例。
    #[test]
    fn candidate_grid_covers_combinations() {
        assert_eq!(keep_ranks(20), [20, 30, 40]);
        assert_eq!(keep_ranks(150), [150, 200]);
        assert_eq!(keep_ranks(1), [1, 2]);
        let grid = candidates(20);
        assert_eq!(grid.len(), 36);
        assert_eq!(
            grid[0],
            Candidate {
                etf_weight: 0,
                max_weight: None,
                keep_rank: 20
            }
        );
        assert_eq!(grid[1].etf_weight, 30);
        assert_eq!(grid[4].max_weight, Some(10));
        assert_eq!(grid[12].keep_rank, 30);
    }

    /// 每年用前兩年挑夏普值最高的組合套用到這一年；訓練期間不足 2 期（沒有夏普值）的年份略過；
    /// 串起來的期數只含樣本外年度，並附不調參數與 0050 的同期報酬。
    #[test]
    fn walk_forward_picks_best_training_sharpe_per_year() {
        let grid = [
            Candidate {
                etf_weight: 0,
                max_weight: None,
                keep_rank: 20,
            },
            Candidate {
                etf_weight: 50,
                max_weight: Some(10),
                keep_rank: 30,
            },
        ];
        // 2020～2021 第一組波動大、第二組穩定；2022 起第一組變穩定、第二組變差。
        let steady = [2.0, 3.0, 2.0, 3.0];
        let swing = [10.0, -8.0, 9.0, -7.0];
        let first: Vec<f64> = [swing, swing, steady, steady, steady].concat();
        let second: Vec<f64> = [steady, steady, swing, [-5.0, -6.0, -5.0, -6.0], swing].concat();
        let tracks = [track(2020, &first), track(2020, &second)];
        let baseline = track(2020, &[1.0; 20]);
        let result = walk_forward(
            &grid,
            &tracks,
            &baseline,
            day(2020, 1, 2),
            day(2024, 12, 31),
            2,
            60,
        );
        let years: Vec<(i32, u8)> = result
            .years
            .iter()
            .map(|year| (year.year, year.etf_weight))
            .collect();
        // 2022：看 2020～2021 選第二組；2023：看 2021～2022，第二組的穩定被 2022 拉下，選第一組。
        assert_eq!(years, [(2022, 50), (2023, 0), (2024, 0)]);
        let first_year = &result.years[0];
        assert_eq!(first_year.max_weight, Some(10));
        assert_eq!(first_year.keep_rank, 30);
        assert_eq!(first_year.periods, 4);
        let swing_return = (1.10 * 0.92 * 1.09 * 0.93 - 1.0) * 100.0;
        assert_eq!(first_year.return_after_costs, round2(swing_return));
        assert_eq!(
            first_year.baseline_return_after_costs,
            round2((1.01_f64.powi(4) - 1.0) * 100.0)
        );
        assert_eq!(
            first_year.etf_return,
            first_year.baseline_return_after_costs.into()
        );
        assert_eq!(result.chosen.rows.len(), 12);
        assert_eq!(result.chosen.costs.len(), 12);
        assert_eq!(result.baseline.rows.len(), 12);
        assert_eq!(result.chosen.rows[0].date, day(2022, 1, 2));

        // 訓練期間只有 1 期：沒有夏普值，全部略過。
        let short = [track(2020, &[1.0]), track(2020, &[2.0])];
        let result = walk_forward(
            &grid,
            &short,
            &baseline,
            day(2020, 1, 2),
            day(2021, 12, 31),
            1,
            60,
        );
        assert!(result.years.is_empty());
        assert!(result.chosen.rows.is_empty());
    }
}
