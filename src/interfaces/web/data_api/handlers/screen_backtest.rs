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
//! 市值加權，和 0050 一樣偏重大型股）或其平方根加權；`max_weight` 限制單檔權重，超出的分給其他持股，
//! 檔數太少分不完時放現金。
//!
//! 設 `stop_loss`／`trailing_stop` 時，持有期間每天以還原收盤檢查，觸發就提前賣出放現金（見
//! [`stops`]）。
//!
//! 子模組：[`engine`]（驗證條件、查候選股、決定持股與扣成本的回測引擎，各端點共用）、[`attribution`]（相對類 0050 基準的報酬歸因端點）、[`sql`]（回測 SQL）、[`portfolio`]（每期持股與交易成本）、[`summary`]（績效彙總）、
//! [`mod@contributors`]（個股貢獻）、[`current`]（目前持股）、[`offsets`]（換股日偏移穩健度端點）、[`frontier`]（0050 核心部位
//! 比例前緣端點）、[`gateway`]（策略換股通知的 adapter）、[`preset`]（具名策略 `preset=名稱` 與清單端點）、[`stops`]（期間停損）、[`track`]（具名策略實盤追蹤端點）、[`walk_forward`]（滾動前推最佳化端點）。

mod attribution;
mod contributors;
mod current;
mod engine;
mod frontier;
mod gateway;
mod offsets;
mod portfolio;
mod preset;
mod sql;
mod stops;
mod summary;
mod track;
mod walk_forward;

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;

pub(crate) use self::attribution::{
    __path_screen_backtest_attribution, screen_backtest_attribution,
};
pub(crate) use self::frontier::{__path_screen_backtest_frontier, screen_backtest_frontier};
pub use self::gateway::BacktestStrategyGateway;
pub(crate) use self::offsets::{__path_screen_backtest_offsets, screen_backtest_offsets};
pub(crate) use self::preset::{
    __path_strategy_presets, BacktestQuery, strategy_presets, validate_strategy_query,
};
pub(crate) use self::track::{__path_strategy_track, strategy_track};
pub(crate) use self::walk_forward::{
    __path_screen_backtest_walk_forward, screen_backtest_walk_forward,
};

use self::contributors::{Contribution, LISTED_CONTRIBUTORS, contributors};
use self::current::{current_holdings, exited, returns_to_date, stock_names};
use self::engine::{Backtest, Raw, Simulation};
use self::portfolio::{PeriodCost, period};
use self::summary::{summarize, yearly};
use super::{database_error, error_response, round2, round4};
use crate::interfaces::web::data_api::dto::{
    BacktestContributor, BacktestContributors, ErrorBody, ScreenBacktestParams,
    ScreenBacktestResponse,
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

/// `exclude_industry` 最多幾個產業。
const MAX_EXCLUDED_INDUSTRIES: usize = 10;

/// 候選股可用的排序方式（`rank_by`），第一個為預設；字面值與 [`sql`] 回測 SQL 的 `$16` 對應。
const RANK_BY: [&str; 6] = [
    "rs_score",
    "trade_value",
    "foreign_streak",
    "trust_streak",
    "revenue_yoy",
    "operating_margin",
];

/// [`BACKTEST_SQL`](sql::BACKTEST_SQL) 的一列：換股日、基準與依相對強度由高到低的候選股（與報酬一一對應）。
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
    /// 策略部位放現金的比例（0–1）：設 `max_weight` 且檔數 × 上限不到 100% 時才大於 0。
    cash: f64,
    /// 各持股這期是否在持有期間停損賣出，與 `symbols` 一一對應；空的代表都沒有（見 [`stops`]）。
    stopped: Vec<bool>,
    /// 各持股的產業編號（`stocks.stock_industry_id`），與 `symbols` 一一對應。
    industries: Vec<i32>,
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
pub(crate) async fn screen_backtest(BacktestQuery(params): BacktestQuery) -> Response {
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
    let exits = current
        .as_ref()
        .map(|(row, _)| exited(row, rows.last()))
        .unwrap_or_default();
    let symbols: Vec<&str> = best
        .iter()
        .chain(&worst)
        .map(|item| item.symbol.as_str())
        .chain(
            current
                .iter()
                .flat_map(|(row, _)| row.symbols.iter().map(String::as_str)),
        )
        .chain(exits.iter().map(|(symbol, _)| symbol.as_str()))
        .collect();
    let names = match stock_names(&symbols).await {
        Ok(names) => names,
        Err(error) => return database_error(error),
    };
    let contributor = |item: Contribution| BacktestContributor {
        name: names.get(&item.symbol).cloned(),
        symbol: item.symbol,
        periods_held: item.periods_held,
        contribution: item.contribution,
        avg_return: item.avg_return,
    };
    let previous = rows.last();
    let returns = match &current {
        Some((row, _)) => returns_to_date(row, backtest.to, elapsed_days.unwrap_or(0)).await,
        None => std::collections::HashMap::new(),
    };
    let current = current.map(|(row, cost)| {
        current_holdings(
            (row, cost),
            previous,
            elapsed_days.unwrap_or(0),
            hold_days,
            &names,
            &returns,
        )
    });

    Json(ScreenBacktestResponse {
        preset: params.preset.clone(),
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
        max_weight: backtest.max_weight,
        stop_loss: params.stop_loss,
        trailing_stop: params.trailing_stop,
        industry: params.industry.clone(),
        exclude_industry: (!backtest.excluded_industries.is_empty())
            .then(|| backtest.excluded_industries.join(",")),
        min_rs_score: params.min_rs_score,
        max_rs_score: params.max_rs_score,
        min_foreign_streak: params.min_foreign_streak,
        min_trust_streak: params.min_trust_streak,
        min_revenue_yoy_percent: params.min_revenue_yoy_percent,
        min_revenue_yoy_streak: params.min_revenue_yoy_streak,
        min_revenue_yoy_3m_percent: params.min_revenue_yoy_3m_percent,
        min_eps: params.min_eps,
        min_roe_percent: params.min_roe_percent,
        min_gross_margin_percent: params.min_gross_margin_percent,
        min_operating_margin_percent: params.min_operating_margin_percent,
        margins_yoy_up: params.margins_yoy_up.unwrap_or(false),
        revenue_record: params.revenue_record.unwrap_or(false),
        max_debt_ratio_percent: params.max_debt_ratio_percent,
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

#[cfg(test)]
mod tests;
