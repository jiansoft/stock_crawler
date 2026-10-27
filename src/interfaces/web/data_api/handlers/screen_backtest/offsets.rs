//! `/stocks/screen/backtest/offsets` handler：同一組條件換不同的換股日偏移重跑，看績效分布。
//!
//! 每隔 `hold_days` 個交易日換股的回測，結果會受「剛好在哪幾天換股」影響：同樣的條件，起日之後
//! 晚幾天開始換股，年化報酬可能差上一倍。這裡把偏移 0 到 `hold_days` − 1 都跑一遍（超過
//! [`MAX_OFFSET_SAMPLES`] 個時平均抽樣），回傳每個偏移的彙總與年化報酬、夏普值、最大回撤的分布，
//! 波動率與 0050 同期的對照，以及勝過 0050 的偏移比例；分布的中位數比任何單一偏移更能代表這組條件。另附依年度的分布，
//! 看策略在哪些年份穩定勝過 0050；指定 `split` 時另把各期依換股日分成樣本內、樣本外，各算一次分布。

use std::collections::BTreeMap;

use chrono::NaiveDate;

use super::limit::run_limited;
use axum::{
    Json,
    response::{IntoResponse, Response},
};

use super::portfolio::PeriodCost;
use super::preset::BacktestQuery;
use super::summary::{summarize, yearly};
use super::{Backtest, ETF_BENCHMARK, PeriodRow, Simulation};
use crate::interfaces::web::data_api::dto::{
    BacktestOffsetDistribution, BacktestOffsetResult, BacktestOffsetYear, BacktestStat,
    BacktestSummary, BacktestYear, ErrorBody, ScreenBacktestOffsetsResponse, ScreenBacktestParams,
};
use crate::interfaces::web::data_api::handlers::{database_error, round2, round4};

/// 最多跑幾個偏移；`hold_days` 超過時在 0～`hold_days` − 1 之間平均抽樣。
const MAX_OFFSET_SAMPLES: u16 = 20;

/// 同時執行的回測查詢數，避免一次占滿連線池。
pub(super) const CONCURRENCY: usize = 4;

/// 同一組條件換不同的換股日偏移重跑，回傳每個偏移的績效彙總與分布。
///
/// 參數與 `/stocks/screen/backtest` 相同，`rebalance_offset` 不使用：`hold_days` 不超過 20 時跑全部
/// 偏移，否則在 0～`hold_days` − 1 之間平均取 20 個。
///
/// # Errors
///
/// 參數驗證與 `/stocks/screen/backtest` 相同（422）；迄日之前沒有相對強度資料回 404；驗證失敗回 401；
/// 資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/stocks/screen/backtest/offsets", tag = "data-api", params(ScreenBacktestParams), responses((status = 200, body = ScreenBacktestOffsetsResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn screen_backtest_offsets(BacktestQuery(params): BacktestQuery) -> Response {
    let backtest = match Backtest::prepare(&params).await {
        Ok(backtest) => backtest.period_end_only(),
        Err(response) => return *response,
    };
    let backtest = &backtest;
    let hold_days = backtest.hold_days;
    let split = backtest.split;
    let simulated = match simulate_offsets(backtest).await {
        Ok(simulated) => simulated,
        Err(error) => return database_error(error),
    };
    let (before_split, after_split) = split_distributions(
        simulated
            .iter()
            .filter_map(|run| run.halves.clone())
            .collect(),
    );
    let (results, years): (Vec<BacktestOffsetResult>, Vec<Vec<BacktestYear>>) = simulated
        .into_iter()
        .map(|run| (run.result, run.years))
        .unzip();

    Json(ScreenBacktestOffsetsResponse {
        from: backtest.from.to_string(),
        to: backtest.to.to_string(),
        market: backtest.market.to_owned(),
        hold_days,
        limit: backtest.limit,
        rank_by: backtest.rank_by.to_owned(),
        etf_weight: backtest.etf_weight,
        weighting: backtest.weighting.as_str().to_owned(),
        max_weight: backtest.max_weight,
        stop_loss: backtest.params.stop_loss,
        trailing_stop: backtest.params.trailing_stop,
        fee_discount: backtest.fee_discount,
        execution_model: backtest.execution.as_str().to_owned(),
        slippage_bps: backtest.slippage_bps,
        drawdown_basis: "period_end".to_owned(),
        etf_benchmark: ETF_BENCHMARK.to_owned(),
        distribution: distribution(&results),
        years: offset_years(&years),
        split: split.map(|date| date.to_string()),
        before_split,
        after_split,
        offsets: results,
    })
    .into_response()
}

/// 一個偏移的回測結果：整段彙總、依年度彙總與樣本內外的彙總。
pub(super) struct OffsetRun {
    pub(super) result: BacktestOffsetResult,
    years: Vec<BacktestYear>,
    halves: Option<(BacktestOffsetResult, BacktestOffsetResult)>,
}

/// 對 [`sample_offsets`] 的每個換股日偏移各跑一次回測，依偏移由小到大回傳；偏移端點與策略優勢
/// 滾動檢查（[`super::gateway`]）共用。
pub(super) async fn simulate_offsets(
    backtest: &Backtest<'_>,
) -> Result<Vec<OffsetRun>, sqlx::Error> {
    let hold_days = backtest.hold_days;
    let split = backtest.split;
    run_limited(sample_offsets(hold_days), |offset| async move {
        backtest
            .simulate(offset)
            .await
            .map(|Simulation { rows, costs, .. }| {
                let halves = split_results(offset, &rows, &costs, hold_days, split);
                OffsetRun {
                    result: BacktestOffsetResult {
                        excluded_cross_boundary_periods: excluded_periods(halves.as_ref()),
                        rebalance_offset: offset,
                        summary: summarize(&rows, &costs, hold_days),
                    },
                    years: yearly(&rows, &costs),
                    halves,
                }
            })
    })
    .await
}

/// 依分割日把各期分成換股日與出場日皆早於 `split` 的樣本內與換股日不早於 `split` 的樣本外，各自彙總；
/// 跨越分割日的期數（換股日早於 `split` 但出場日在 `split`（含）之後）不計入樣本內。
/// 沒有分割日回 `None`。`rows` 依換股日由舊到新，`costs` 與之一一對應。
pub(super) fn split_results(
    offset: u16,
    rows: &[PeriodRow],
    costs: &[PeriodCost],
    hold_days: u16,
    split: Option<NaiveDate>,
) -> Option<(BacktestOffsetResult, BacktestOffsetResult)> {
    let split = split?;
    let in_sample_end =
        rows.partition_point(|row| row.exit_date.map_or(row.date < split, |exit| exit < split));
    let out_sample_start = rows.partition_point(|row| row.date < split);
    let cross_boundary_count = out_sample_start.saturating_sub(in_sample_end);
    if cross_boundary_count > 0 {
        tracing::info!(
            "換股日偏移 {offset}：排除 {cross_boundary_count} 期跨越分割日 {split} 的期數"
        );
    }
    let result = |range: std::ops::Range<usize>| BacktestOffsetResult {
        excluded_cross_boundary_periods: cross_boundary_count,
        rebalance_offset: offset,
        summary: summarize(&rows[range.clone()], &costs[range], hold_days),
    };
    Some((
        result(0..in_sample_end),
        result(out_sample_start..rows.len()),
    ))
}

/// 跨越分割日、不計入樣本內外的期數；沒有分割日為 0。
pub(super) fn excluded_periods(
    halves: Option<&(BacktestOffsetResult, BacktestOffsetResult)>,
) -> usize {
    halves.map_or(0, |(before, _)| before.excluded_cross_boundary_periods)
}

/// 各偏移樣本內、樣本外彙總的分布；沒有分割日（`halves` 為空）時都是 `None`。
pub(super) fn split_distributions(
    halves: Vec<(BacktestOffsetResult, BacktestOffsetResult)>,
) -> (
    Option<BacktestOffsetDistribution>,
    Option<BacktestOffsetDistribution>,
) {
    if halves.is_empty() {
        return (None, None);
    }
    let (before, after): (Vec<_>, Vec<_>) = halves.into_iter().unzip();
    (Some(distribution(&before)), Some(distribution(&after)))
}

/// 要跑的換股日偏移：`hold_days` 不超過 [`MAX_OFFSET_SAMPLES`] 時為全部，否則平均抽樣（含 0）。
pub(super) fn sample_offsets(hold_days: u16) -> Vec<u16> {
    if hold_days <= MAX_OFFSET_SAMPLES {
        return (0..hold_days).collect();
    }
    (0..MAX_OFFSET_SAMPLES)
        .map(|k| (u32::from(k) * u32::from(hold_days) / u32::from(MAX_OFFSET_SAMPLES)) as u16)
        .collect()
}

/// 各偏移扣成本後年化報酬、夏普值、最大回撤與波動率（與 0050 同期）的分布，以及勝過 0050 的偏移
/// 比例（回撤以較淺為勝）。
pub(super) fn distribution(results: &[BacktestOffsetResult]) -> BacktestOffsetDistribution {
    let field = |pick: fn(&BacktestSummary) -> Option<f64>| {
        stat(results.iter().filter_map(|result| pick(&result.summary)))
    };
    let beat = |pick: fn(&BacktestSummary) -> (Option<f64>, Option<f64>)| {
        let pairs: Vec<(f64, f64)> = results
            .iter()
            .filter_map(|result| match pick(&result.summary) {
                (Some(strategy), Some(etf)) => Some((strategy, etf)),
                _ => None,
            })
            .collect();
        (!pairs.is_empty()).then(|| {
            round4(
                pairs
                    .iter()
                    .filter(|(strategy, etf)| strategy > etf)
                    .count() as f64
                    / pairs.len() as f64,
            )
        })
    };
    BacktestOffsetDistribution {
        annualized_return_after_costs: field(|s| s.annualized_return_after_costs),
        sharpe_ratio: field(|s| s.sharpe_ratio),
        max_drawdown_after_costs: field(|s| s.max_drawdown_after_costs),
        etf_annualized_return: field(|s| s.etf_annualized_return),
        etf_sharpe_ratio: field(|s| s.etf_sharpe_ratio),
        volatility: field(|s| s.volatility),
        etf_volatility: field(|s| s.etf_volatility),
        etf_max_drawdown: field(|s| s.etf_max_drawdown),
        beat_etf_return_rate: beat(|s| (s.annualized_return_after_costs, s.etf_annualized_return)),
        beat_etf_sharpe_rate: beat(|s| (s.sharpe_ratio, s.etf_sharpe_ratio)),
        beat_etf_drawdown_rate: beat(|s| (s.max_drawdown_after_costs, s.etf_max_drawdown)),
    }
}

/// 各偏移的依年度彙總（每個偏移一組，見 [`yearly`]）合併成每年的分布，由舊到新。
pub(super) fn offset_years(per_offset: &[Vec<BacktestYear>]) -> Vec<BacktestOffsetYear> {
    // 年度 → (扣成本後報酬, 0050 報酬, 勝過 0050 的偏移數)
    let mut by_year: BTreeMap<i32, (Vec<f64>, Vec<f64>, usize)> = BTreeMap::new();
    for year in per_offset.iter().flatten() {
        let entry = by_year.entry(year.year).or_default();
        entry.0.push(year.total_return_after_costs);
        if let Some(etf) = year.etf_total_return {
            entry.1.push(etf);
            entry.2 += usize::from(year.total_return_after_costs > etf);
        }
    }
    by_year
        .into_iter()
        .map(|(year, (strategy, etf, beat))| BacktestOffsetYear {
            year,
            offsets: strategy.len() as i64,
            return_after_costs: stat(strategy.iter().copied()),
            beat_etf_rate: (!etf.is_empty()).then(|| round4(beat as f64 / etf.len() as f64)),
            etf_return: stat(etf.into_iter()),
        })
        .collect()
}

/// 一組數值的最小、四分位、中位數、平均與最大值（四分位以線性內插）；沒有數值回 `None`。
pub(super) fn stat(values: impl Iterator<Item = f64>) -> Option<BacktestStat> {
    let mut values: Vec<f64> = values.collect();
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let quantile = |q: f64| {
        let position = q * (values.len() - 1) as f64;
        let lower = position.floor() as usize;
        let upper = position.ceil() as usize;
        values[lower] + (values[upper] - values[lower]) * (position - lower as f64)
    };
    Some(BacktestStat {
        min: round2(values[0]),
        p25: round2(quantile(0.25)),
        median: round2(quantile(0.5)),
        mean: round2(values.iter().sum::<f64>() / values.len() as f64),
        p75: round2(quantile(0.75)),
        max: round2(values[values.len() - 1]),
    })
}
