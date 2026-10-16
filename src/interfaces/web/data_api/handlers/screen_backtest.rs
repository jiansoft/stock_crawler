//! `/stocks/screen/backtest` handler：依每日留存的條件定期選股，回測等權持有的報酬。
//!
//! 每隔 `hold_days` 個交易日換股一次：在換股日從相對強度母體（上市櫃普通股，見
//! `daily_relative_strength`）挑出符合條件的股票，依 `rank_by`（預設相對強度分數）由高到低取前
//! `limit` 檔，
//! 收盤等權買進、持有 `hold_days` 個交易日；該期報酬取 `daily_forward_return` 的同天期還原報酬，
//! 期間內下市的股票算到最後一筆收盤。只用當天就已知道的資料（相對強度、法人連續買超、技術訊號），
//! 沒有前視偏誤；相對強度母體看當時有沒有在交易，沒有存活者偏差。
//!
//! 交易成本依換股時實際賣出、買進的比例扣除：買賣各付手續費 0.1425%（乘上 `fee_discount`），
//! 賣出另付證交稅 0.3%；續抱股票漲跌後調回目標權重（再平衡）的買賣也計入。
//!
//! 有設 `etf_weight` 時，組合固定以該比例持有 0050、其餘才跑策略（核心／衛星配置），彙總與每期報酬
//! 都是混合後的組合報酬，每期調回固定比例的買賣也扣成本（0050 證交稅 0.1%）；個股貢獻與持股仍只看
//! 策略部位。
//!
//! `weighting` 決定策略部位的持股權重：預設等權，也可依換股日前 20 個交易日的平均成交金額（接近
//! 市值加權，和 0050 一樣偏重大型股）或其平方根加權。
//!
//! 子模組：[`sql`]（回測 SQL）、[`portfolio`]（每期持股與交易成本）、[`summary`]（績效彙總）、
//! [`contributors`]（個股貢獻）、[`offsets`]（換股日偏移穩健度端點）、[`frontier`]（0050 核心部位
//! 比例前緣端點）。

mod contributors;
mod frontier;
mod offsets;
mod portfolio;
mod sql;
mod summary;

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;

pub(crate) use self::frontier::{__path_screen_backtest_frontier, screen_backtest_frontier};
pub(crate) use self::offsets::{__path_screen_backtest_offsets, screen_backtest_offsets};

use self::contributors::{Contribution, LISTED_CONTRIBUTORS, contributors};
use self::portfolio::{
    PeriodCost, Weighting, blend_etf, period, select_holdings, trading_costs, weigh,
};
use self::sql::{BACKTEST_SQL, BACKTEST_WITH_CHIP_SQL, LATEST_RS_DATE_SQL};
use self::summary::{summarize, yearly};
use super::signal_history::{validate_rs_range, validate_streaks};
use super::{
    database_error, error_response, industry_error, market_id_for_stocks, parse_optional_date,
    round2, round4, signal_filter_error,
};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    BacktestContributor, BacktestContributors, BacktestCurrent, BacktestHolding, ErrorBody,
    ScreenBacktestParams, ScreenBacktestResponse,
};

/// 可選的持有天數，與 `daily_forward_return` 的報酬欄位對應。
const HOLD_DAYS: [u16; 5] = [5, 20, 60, 120, 240];

/// 預設回測期間：迄日往前 1,095 天（約三年）。
const DEFAULT_PERIOD_DAYS: i64 = 1095;

/// 回測期間上限（日曆天，含起訖）。
const MAX_PERIOD_DAYS: i64 = 3653;

/// 可投資的大盤基準：元大台灣 50（0050），SQL 內以字面值寫死，改這裡要一併改 [`sql`] 的回測 SQL。
const ETF_BENCHMARK: &str = "0050";

/// 大盤濾網可選的均線天數（交易日）。
const MARKET_FILTER_DAYS: [u16; 4] = [20, 60, 120, 240];

/// `keep_rank` 上限：續抱的股票最多可以落到符合條件者的第幾名；設了 `max_per_industry` 時
/// 也是每期讀取的候選檔數（同產業額滿後要往後補）。
const MAX_KEEP_RANK: u8 = 200;

/// `min_avg_trade_value` 上限（百萬元）：日均成交 1,000 億元，已超過全市場最大的個股。
const MAX_TRADE_VALUE_MILLION: u32 = 100_000;

/// 候選股可用的排序方式（`rank_by`），第一個為預設；字面值與 [`sql`] 回測 SQL 的 `$16` 對應。
const RANK_BY: [&str; 4] = ["rs_score", "trade_value", "foreign_streak", "trust_streak"];

/// [`BACKTEST_SQL`] 的一列：換股日、基準與依相對強度由高到低的候選股（與報酬一一對應）。
#[derive(Debug, Clone, Default, PartialEq, sqlx::FromRow)]
struct CandidateRow {
    date: NaiveDate,
    /// 最後一個換股日、還沒滿持有天數（沒有報酬）；代表策略目前的持股。
    pending: bool,
    /// 換股日之後到迄日經過的交易日數。
    elapsed_days: i64,
    /// 大盤濾網是否通過；沒有設濾網時恆為 true。
    market_ok: bool,
    benchmark_return: Option<f64>,
    etf_return: Option<f64>,
    candidates: Vec<String>,
    candidate_returns: Vec<Option<f64>>,
    /// 候選股的產業編號（`stocks.stock_industry_id`），與 `candidates` 一一對應。
    candidate_industries: Vec<i32>,
    /// 候選股的平均成交金額（元），與 `candidates` 一一對應；沒有用到成交金額時為 `NULL`。
    candidate_trade_values: Vec<Option<f64>>,
    /// 相對強度不低於續抱門檻的母體股票（依相對強度由高到低）與報酬、產業編號；沒設門檻為空。
    hold_symbols: Vec<String>,
    hold_returns: Vec<Option<f64>>,
    hold_industries: Vec<i32>,
    /// 續抱名單的平均成交金額（元）；只有依成交金額加權時才查。
    hold_trade_values: Vec<Option<f64>>,
}

/// 一期的結果；`symbols` 是全部持股，依相對強度由高到低。
#[derive(Debug, Clone, Default, PartialEq)]
struct PeriodRow {
    date: NaiveDate,
    /// 還沒滿持有天數的最後一期（見 [`CandidateRow::pending`]）。
    pending: bool,
    /// 大盤濾網沒過、這期被迫空手。
    market_filtered: bool,
    holdings: i64,
    avg_return: Option<f64>,
    benchmark_return: Option<f64>,
    /// [`ETF_BENCHMARK`] 同期的還原報酬；尚未上市或缺資料為 `None`。
    etf_return: Option<f64>,
    symbols: Vec<String>,
    /// 各持股這期的報酬，與 `symbols` 一一對應；之後完全沒有收盤的為 `None`。
    returns: Vec<Option<f64>>,
    /// 各持股換股日的平均成交金額（元），與 `symbols` 一一對應。
    trade_values: Vec<Option<f64>>,
    /// 各持股的相對權重（未正規化），與 `symbols` 一一對應；空的代表等權（見
    /// [`portfolio::weight_at`]）。
    weights: Vec<f64>,
}

/// 依條件每隔 `hold_days` 個交易日換股，回測等權持有前 `limit` 檔（相對強度由高到低）的報酬，
/// 並與同市場全部母體股票的平均比較。
///
/// # Errors
///
/// 日期格式錯誤、起日晚於迄日、期間超過 3,653 天、`hold_days` 不是 5／20／60／120／240、`limit`
/// 不在 1–100、`fee_discount` 不在 0–1、相對強度或連續買超天數超出範圍、訊號來源／種類不在固定清單
/// 或產業名稱不存在回 422；
/// 迄日之前沒有相對強度資料回 404；驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/stocks/screen/backtest", tag = "data-api", params(ScreenBacktestParams), responses((status = 200, body = ScreenBacktestResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn screen_backtest(Query(params): Query<ScreenBacktestParams>) -> Response {
    let backtest = match Backtest::prepare(&params).await {
        Ok(backtest) => backtest,
        Err(response) => return *response,
    };
    let hold_days = backtest.hold_days;
    let rebalance_offset = params.rebalance_offset.unwrap_or(0);
    if rebalance_offset >= hold_days {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "rebalance_offset 必須介於 0 至 hold_days − 1",
        );
    }
    let Simulation {
        rows,
        costs,
        current,
        elapsed_days,
    } = match backtest.simulate(rebalance_offset).await {
        Ok(simulation) => simulation,
        Err(error) => return database_error(error),
    };
    let (held_symbols, best, worst) = contributors(&rows, LISTED_CONTRIBUTORS);
    let symbols: Vec<&str> = best
        .iter()
        .chain(&worst)
        .map(|item| item.symbol.as_str())
        .chain(
            current
                .iter()
                .flat_map(|(row, _)| row.symbols.iter().map(String::as_str)),
        )
        .collect();
    let names: Vec<(String, String)> = match sqlx::query_as(STOCK_NAMES_SQL)
        .bind(&symbols)
        .fetch_all(database::get_connection())
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    let names: std::collections::HashMap<String, String> = names.into_iter().collect();
    let contributor = |item: Contribution| BacktestContributor {
        name: names.get(&item.symbol).cloned(),
        symbol: item.symbol,
        periods_held: item.periods_held,
        contribution: item.contribution,
        avg_return: item.avg_return,
    };
    let current = current.map(|(row, cost)| {
        let elapsed = elapsed_days.unwrap_or(0);
        let total: f64 = (0..row.symbols.len())
            .map(|index| portfolio::weight_at(&row, index))
            .sum();
        let weights: Vec<f64> = (0..row.symbols.len())
            .map(|index| round2(portfolio::weight_at(&row, index) / total * 100.0))
            .collect();
        BacktestCurrent {
            date: row.date.to_string(),
            days_elapsed: elapsed,
            days_to_next_rebalance: (i64::from(hold_days) - elapsed).max(0),
            market_filtered: row.market_filtered,
            turnover: round4(cost.turnover),
            holdings: row
                .symbols
                .into_iter()
                .zip(weights)
                .map(|(symbol, weight)| BacktestHolding {
                    name: names.get(&symbol).cloned(),
                    symbol,
                    weight,
                })
                .collect(),
        }
    });

    Json(ScreenBacktestResponse {
        from: backtest.from.to_string(),
        to: backtest.to.to_string(),
        market: backtest.market.to_owned(),
        hold_days,
        limit: backtest.limit,
        keep_rank: backtest.keep_rank,
        market_filter_ma: params.market_filter_ma,
        market_filter_breadth: params.market_filter_breadth,
        hold_min_rs_score: params.hold_min_rs_score,
        max_per_industry: params.max_per_industry,
        rebalance_offset,
        etf_weight: backtest.etf_weight,
        min_avg_trade_value: params.min_avg_trade_value,
        rank_by: backtest.rank_by.to_owned(),
        weighting: backtest.weighting.as_str().to_owned(),
        industry: params.industry.clone(),
        min_rs_score: params.min_rs_score,
        max_rs_score: params.max_rs_score,
        min_foreign_streak: params.min_foreign_streak,
        min_trust_streak: params.min_trust_streak,
        indicator: params.indicator.clone(),
        kind: params.kind.clone(),
        fee_discount: backtest.fee_discount,
        adjusted: true,
        etf_benchmark: ETF_BENCHMARK.to_owned(),
        summary: summarize(&rows, &costs, hold_days),
        years: yearly(&rows, &costs),
        contributors: BacktestContributors {
            held_symbols: held_symbols as i64,
            best: best.into_iter().map(contributor).collect(),
            worst: worst.into_iter().map(contributor).collect(),
        },
        current,
        periods: rows.into_iter().zip(costs).map(period).collect(),
    })
    .into_response()
}

/// 驗證過的回測條件（換股日偏移除外，由呼叫端決定）；`/stocks/screen/backtest` 與
/// `/stocks/screen/backtest/offsets` 共用。
struct Backtest<'a> {
    params: &'a ScreenBacktestParams,
    from: NaiveDate,
    /// 迄日（含）之前最近一個有相對強度的交易日。
    to: NaiveDate,
    market: &'a str,
    market_id: i32,
    hold_days: u16,
    limit: u8,
    keep_rank: u8,
    fee_discount: f64,
    /// 候選股排序方式（[`RANK_BY`] 其中之一）。
    rank_by: &'a str,
    /// 0050 核心部位的比例（%，0 為純策略）。
    etf_weight: u8,
    /// 策略部位的持股加權方式。
    weighting: Weighting,
}

/// 一次回測混合 0050 之前的各期結果（含還沒滿持有天數的最後一期）；同一組候選股可依不同的
/// `etf_weight` 重複混合（見 [`Raw::finish`]）。
#[derive(Clone)]
struct Raw {
    rows: Vec<PeriodRow>,
    costs: Vec<PeriodCost>,
    /// 最後一期還沒滿持有天數時，換股日之後到迄日經過的交易日數。
    elapsed_days: Option<i64>,
    /// 手續費折扣，0050 再平衡成本也用這個折扣。
    fee_discount: f64,
}

impl Raw {
    /// 以 `etf_weight`（%）混合 0050 核心部位，並把還沒滿持有天數的最後一期移到
    /// [`Simulation::current`]。
    fn finish(mut self, etf_weight: u8) -> Simulation {
        blend_etf(
            &mut self.rows,
            &mut self.costs,
            f64::from(etf_weight) / 100.0,
            self.fee_discount,
        );
        let current = split_current(&mut self.rows, &mut self.costs);
        Simulation {
            rows: self.rows,
            costs: self.costs,
            current,
            elapsed_days: self.elapsed_days,
        }
    }
}

/// 一次回測的各期結果與目前持股。
struct Simulation {
    /// 已滿持有天數的各期持股與報酬。
    rows: Vec<PeriodRow>,
    /// 與 `rows` 一一對應的換手與扣成本後報酬。
    costs: Vec<PeriodCost>,
    /// 最後一次換股、還沒滿持有天數的持股（見 [`split_current`]）。
    current: Option<(PeriodRow, PeriodCost)>,
    /// `current` 換股日之後到迄日經過的交易日數。
    elapsed_days: Option<i64>,
}

impl<'a> Backtest<'a> {
    /// 驗證參數、確認產業存在，並決定實際的起訖日；失敗時回錯誤回應（422／404／500）。
    async fn prepare(params: &'a ScreenBacktestParams) -> Result<Self, Box<Response>> {
        let unprocessable =
            |message: &str| Box::new(error_response(StatusCode::UNPROCESSABLE_ENTITY, message));
        let (from, to) = match (
            parse_optional_date(params.from.as_deref()),
            parse_optional_date(params.to.as_deref()),
        ) {
            (Ok(from), Ok(to)) => (from, to),
            (Err(message), _) | (_, Err(message)) => return Err(unprocessable(message)),
        };
        let hold_days = params.hold_days.unwrap_or(20);
        if !HOLD_DAYS.contains(&hold_days) {
            return Err(unprocessable("hold_days 必須為 5、20、60、120 或 240"));
        }
        let limit = params.limit.unwrap_or(20);
        if !(1..=100).contains(&limit) {
            return Err(unprocessable("limit 必須介於 1 至 100"));
        }
        if params
            .market_filter_ma
            .is_some_and(|days| !MARKET_FILTER_DAYS.contains(&days))
        {
            return Err(unprocessable("market_filter_ma 必須為 20、60、120 或 240"));
        }
        if params
            .market_filter_breadth
            .is_some_and(|percent| !(1..=99).contains(&percent))
        {
            return Err(unprocessable("market_filter_breadth 必須介於 1 至 99"));
        }
        if params
            .hold_min_rs_score
            .is_some_and(|score| !(1..=99).contains(&score))
        {
            return Err(unprocessable("hold_min_rs_score 必須介於 1 至 99"));
        }
        if params
            .max_per_industry
            .is_some_and(|count| !(1..=100).contains(&count))
        {
            return Err(unprocessable("max_per_industry 必須介於 1 至 100"));
        }
        if params
            .min_avg_trade_value
            .is_some_and(|value| !(1..=MAX_TRADE_VALUE_MILLION).contains(&value))
        {
            return Err(unprocessable(
                "min_avg_trade_value 必須介於 1 至 100000（百萬元）",
            ));
        }
        let rank_by = params.rank_by.as_deref().unwrap_or(RANK_BY[0]);
        if !RANK_BY.contains(&rank_by) {
            return Err(unprocessable(
                "rank_by 必須為 rs_score、trade_value、foreign_streak 或 trust_streak",
            ));
        }
        let Some(weighting) = Weighting::parse(params.weighting.as_deref().unwrap_or("equal"))
        else {
            return Err(unprocessable(
                "weighting 必須為 equal、trade_value 或 sqrt_trade_value",
            ));
        };
        let etf_weight = params.etf_weight.unwrap_or(0);
        if etf_weight > 100 {
            return Err(unprocessable("etf_weight 必須介於 0 至 100"));
        }
        let keep_rank = params.keep_rank.unwrap_or(limit);
        if !(limit..=MAX_KEEP_RANK).contains(&keep_rank) {
            return Err(unprocessable("keep_rank 必須介於 limit 至 200"));
        }
        let fee_discount = params.fee_discount.unwrap_or(1.0);
        if !(0.0..=1.0).contains(&fee_discount) {
            return Err(unprocessable("fee_discount 必須介於 0 至 1"));
        }
        let market = params.market.as_deref().unwrap_or("all");
        let Some(market_id) = market_id_for_stocks(market) else {
            return Err(unprocessable("market 必須為 all、twse 或 tpex"));
        };
        if let Some(response) =
            signal_filter_error(params.kind.as_deref(), params.indicator.as_deref())
        {
            return Err(Box::new(response));
        }
        if let Err(message) = validate_rs_range(params.min_rs_score, params.max_rs_score) {
            return Err(unprocessable(message));
        }
        if let Err(message) = validate_streaks(params.min_foreign_streak, params.min_trust_streak) {
            return Err(unprocessable(message));
        }
        if let Some(response) = industry_error(params.industry.as_deref()).await {
            return Err(Box::new(response));
        }

        let latest: Option<NaiveDate> = sqlx::query_scalar(LATEST_RS_DATE_SQL)
            .bind(to)
            .fetch_one(database::get_connection())
            .await
            .map_err(|error| Box::new(database_error(error)))?;
        let Some(to) = latest else {
            return Err(Box::new(error_response(
                StatusCode::NOT_FOUND,
                "查無相對強度資料",
            )));
        };
        let from = resolve_backtest_period(from, to).map_err(unprocessable)?;
        Ok(Self {
            params,
            from,
            to,
            market,
            market_id,
            hold_days,
            limit,
            keep_rank,
            fee_discount,
            rank_by,
            etf_weight,
            weighting,
        })
    }

    /// 以換股日偏移 `rebalance_offset` 跑一次回測：混合 `etf_weight` 的 0050 核心部位，並把還沒滿
    /// 持有天數的最後一期移到 [`Simulation::current`]。
    async fn simulate(&self, rebalance_offset: u16) -> Result<Simulation, sqlx::Error> {
        Ok(self.run(rebalance_offset).await?.finish(self.etf_weight))
    }

    /// 以換股日偏移 `rebalance_offset` 查候選股、決定每期持股並扣交易成本，尚未混合 0050。
    async fn run(&self, rebalance_offset: u16) -> Result<Raw, sqlx::Error> {
        let params = self.params;
        let with_chip = params.min_foreign_streak.is_some()
            || params.min_trust_streak.is_some()
            || self.rank_by.ends_with("_streak");
        let mut query = sqlx::query_as(if with_chip {
            BACKTEST_WITH_CHIP_SQL
        } else {
            BACKTEST_SQL
        })
        .bind(self.from)
        .bind(self.to)
        .bind(self.market_id)
        .bind(i32::from(self.hold_days))
        .bind(i64::from(if params.max_per_industry.is_some() {
            MAX_KEEP_RANK
        } else {
            self.keep_rank
        }))
        .bind(params.industry.as_deref())
        .bind(params.min_rs_score.map(i16::from))
        .bind(params.max_rs_score.map(i16::from))
        .bind(params.indicator.as_deref())
        .bind(params.kind.as_deref())
        .bind(params.market_filter_ma.map(i32::from))
        .bind(params.hold_min_rs_score.map(i16::from))
        .bind(params.market_filter_breadth.map(i32::from))
        .bind(i32::from(rebalance_offset))
        .bind(
            params
                .min_avg_trade_value
                .map(|value| i64::from(value) * 1_000_000),
        )
        .bind(self.rank_by)
        .bind(self.weighting.as_str());
        if with_chip {
            query = query
                .bind(params.min_foreign_streak.map(i64::from))
                .bind(params.min_trust_streak.map(i64::from));
        }
        let candidates: Vec<CandidateRow> = query.fetch_all(database::get_connection()).await?;
        let elapsed_days = candidates
            .last()
            .filter(|row| row.pending)
            .map(|row| row.elapsed_days);
        let mut rows = select_holdings(
            candidates,
            usize::from(self.limit),
            usize::from(self.keep_rank),
            params.max_per_industry.map(usize::from),
        );
        weigh(&mut rows, self.weighting);
        let costs = trading_costs(&rows, self.fee_discount);
        Ok(Raw {
            rows,
            costs,
            elapsed_days,
            fee_discount: self.fee_discount,
        })
    }
}

/// 最後一期還沒滿持有天數時把它從各期結果移出，回傳該期持股與換手；其餘期數不受影響。
fn split_current(
    rows: &mut Vec<PeriodRow>,
    costs: &mut Vec<PeriodCost>,
) -> Option<(PeriodRow, PeriodCost)> {
    if !rows.last()?.pending {
        return None;
    }
    Some((rows.pop()?, costs.pop()?))
}

/// 個股貢獻與目前持股的股票名稱。
const STOCK_NAMES_SQL: &str =
    r#"SELECT stock_symbol::text, "Name"::text FROM stocks WHERE stock_symbol = ANY($1)"#;

/// 起日：未指定為迄日往前 [`DEFAULT_PERIOD_DAYS`] 天；晚於迄日或期間超過上限回錯誤訊息。
fn resolve_backtest_period(
    from: Option<NaiveDate>,
    to: NaiveDate,
) -> Result<NaiveDate, &'static str> {
    let from = from.unwrap_or(to - chrono::Duration::days(DEFAULT_PERIOD_DAYS));
    if from > to {
        return Err("from 不可晚於 to");
    }
    if (to - from).num_days() >= MAX_PERIOD_DAYS {
        return Err("回測期間最長 3,653 天");
    }
    Ok(from)
}

#[cfg(test)]
mod tests;
