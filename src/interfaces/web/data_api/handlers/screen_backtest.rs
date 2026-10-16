//! `/stocks/screen/backtest` handler：依每日留存的條件定期選股，回測等權持有的報酬。
//!
//! 每隔 `hold_days` 個交易日換股一次：在換股日從相對強度母體（上市櫃普通股，見
//! `daily_relative_strength`）挑出符合條件的股票，依相對強度分數由高到低取前 `limit` 檔，
//! 收盤等權買進、持有 `hold_days` 個交易日；該期報酬取 `daily_forward_return` 的同天期還原報酬，
//! 期間內下市的股票算到最後一筆收盤。只用當天就已知道的資料（相對強度、法人連續買超、技術訊號），
//! 沒有前視偏誤；相對強度母體看當時有沒有在交易，沒有存活者偏差。
//!
//! 交易成本依換股時實際賣出、買進的比例扣除：買賣各付手續費 0.1425%（乘上 `fee_discount`），
//! 賣出另付證交稅 0.3%；同一檔續抱不計成本，也不計等權再平衡的小額調整。
//!
//! 子模組：[`sql`]（回測 SQL）、[`portfolio`]（每期持股與交易成本）、[`summary`]（績效彙總）、
//! [`contributors`]（個股貢獻）。

mod contributors;
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

use self::contributors::{Contribution, LISTED_CONTRIBUTORS, contributors};
use self::portfolio::{PeriodCost, period, select_holdings, trading_costs};
use self::sql::{BACKTEST_SQL, BACKTEST_WITH_CHIP_SQL, LATEST_RS_DATE_SQL};
use self::summary::{summarize, yearly};
use super::signal_history::{validate_rs_range, validate_streaks};
use super::{
    database_error, error_response, industry_error, market_id_for_stocks, parse_optional_date,
    round2, round4, signal_filter_error,
};
use crate::infra::database;
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

/// [`BACKTEST_SQL`] 的一列：換股日、基準與依相對強度由高到低的候選股（與報酬一一對應）。
#[derive(Debug, Clone, Default, PartialEq, sqlx::FromRow)]
struct CandidateRow {
    date: NaiveDate,
    /// 大盤濾網是否通過；沒有設濾網時恆為 true。
    market_ok: bool,
    benchmark_return: Option<f64>,
    etf_return: Option<f64>,
    candidates: Vec<String>,
    candidate_returns: Vec<Option<f64>>,
    /// 候選股的產業編號（`stocks.stock_industry_id`），與 `candidates` 一一對應。
    candidate_industries: Vec<i32>,
    /// 相對強度不低於續抱門檻的母體股票（依相對強度由高到低）與報酬、產業編號；沒設門檻為空。
    hold_symbols: Vec<String>,
    hold_returns: Vec<Option<f64>>,
    hold_industries: Vec<i32>,
}

/// 一期的結果；`symbols` 是全部持股，依相對強度由高到低。
#[derive(Debug, Clone, Default, PartialEq)]
struct PeriodRow {
    date: NaiveDate,
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
    let (from, to) = match (
        parse_optional_date(params.from.as_deref()),
        parse_optional_date(params.to.as_deref()),
    ) {
        (Ok(from), Ok(to)) => (from, to),
        (Err(message), _) | (_, Err(message)) => {
            return error_response(StatusCode::UNPROCESSABLE_ENTITY, message);
        }
    };
    let hold_days = params.hold_days.unwrap_or(20);
    if !HOLD_DAYS.contains(&hold_days) {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "hold_days 必須為 5、20、60、120 或 240",
        );
    }
    let limit = params.limit.unwrap_or(20);
    if !(1..=100).contains(&limit) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 100");
    }
    if params
        .market_filter_ma
        .is_some_and(|days| !MARKET_FILTER_DAYS.contains(&days))
    {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "market_filter_ma 必須為 20、60、120 或 240",
        );
    }
    if params
        .market_filter_breadth
        .is_some_and(|percent| !(1..=99).contains(&percent))
    {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "market_filter_breadth 必須介於 1 至 99",
        );
    }
    if params
        .hold_min_rs_score
        .is_some_and(|score| !(1..=99).contains(&score))
    {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "hold_min_rs_score 必須介於 1 至 99",
        );
    }
    if params
        .max_per_industry
        .is_some_and(|count| !(1..=100).contains(&count))
    {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "max_per_industry 必須介於 1 至 100",
        );
    }
    let keep_rank = params.keep_rank.unwrap_or(limit);
    if !(limit..=MAX_KEEP_RANK).contains(&keep_rank) {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "keep_rank 必須介於 limit 至 200",
        );
    }
    let fee_discount = params.fee_discount.unwrap_or(1.0);
    if !(0.0..=1.0).contains(&fee_discount) {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "fee_discount 必須介於 0 至 1",
        );
    }
    let market = params.market.as_deref().unwrap_or("all");
    let Some(market_id) = market_id_for_stocks(market) else {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "market 必須為 all、twse 或 tpex",
        );
    };
    if let Some(response) = signal_filter_error(params.kind.as_deref(), params.indicator.as_deref())
    {
        return response;
    }
    if let Err(message) = validate_rs_range(params.min_rs_score, params.max_rs_score) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, message);
    }
    if let Err(message) = validate_streaks(params.min_foreign_streak, params.min_trust_streak) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, message);
    }
    if let Some(response) = industry_error(params.industry.as_deref()).await {
        return response;
    }

    let pool = database::get_connection();
    let latest: Option<NaiveDate> = match sqlx::query_scalar(LATEST_RS_DATE_SQL)
        .bind(to)
        .fetch_one(pool)
        .await
    {
        Ok(value) => value,
        Err(error) => return database_error(error),
    };
    let Some(to) = latest else {
        return error_response(StatusCode::NOT_FOUND, "查無相對強度資料");
    };
    let from = match resolve_backtest_period(from, to) {
        Ok(from) => from,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };

    let with_chip = params.min_foreign_streak.is_some() || params.min_trust_streak.is_some();
    let mut query = sqlx::query_as(if with_chip {
        BACKTEST_WITH_CHIP_SQL
    } else {
        BACKTEST_SQL
    })
    .bind(from)
    .bind(to)
    .bind(market_id)
    .bind(i32::from(hold_days))
    .bind(i64::from(if params.max_per_industry.is_some() {
        MAX_KEEP_RANK
    } else {
        keep_rank
    }))
    .bind(params.industry.as_deref())
    .bind(params.min_rs_score.map(i16::from))
    .bind(params.max_rs_score.map(i16::from))
    .bind(params.indicator.as_deref())
    .bind(params.kind.as_deref())
    .bind(params.market_filter_ma.map(i32::from))
    .bind(params.hold_min_rs_score.map(i16::from))
    .bind(params.market_filter_breadth.map(i32::from));
    if with_chip {
        query = query
            .bind(params.min_foreign_streak.map(i64::from))
            .bind(params.min_trust_streak.map(i64::from));
    }
    let candidates: Vec<CandidateRow> = match query.fetch_all(pool).await {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    let rows = select_holdings(
        candidates,
        usize::from(limit),
        usize::from(keep_rank),
        params.max_per_industry.map(usize::from),
    );
    let costs = trading_costs(&rows, fee_discount);
    let (held_symbols, best, worst) = contributors(&rows, LISTED_CONTRIBUTORS);
    let symbols: Vec<&str> = best
        .iter()
        .chain(&worst)
        .map(|item| item.symbol.as_str())
        .collect();
    let names: Vec<(String, String)> = match sqlx::query_as(STOCK_NAMES_SQL)
        .bind(&symbols)
        .fetch_all(pool)
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

    Json(ScreenBacktestResponse {
        from: from.to_string(),
        to: to.to_string(),
        market: market.to_owned(),
        hold_days,
        limit,
        keep_rank,
        market_filter_ma: params.market_filter_ma,
        market_filter_breadth: params.market_filter_breadth,
        hold_min_rs_score: params.hold_min_rs_score,
        max_per_industry: params.max_per_industry,
        industry: params.industry.clone(),
        min_rs_score: params.min_rs_score,
        max_rs_score: params.max_rs_score,
        min_foreign_streak: params.min_foreign_streak,
        min_trust_streak: params.min_trust_streak,
        indicator: params.indicator.clone(),
        kind: params.kind.clone(),
        fee_discount,
        adjusted: true,
        etf_benchmark: ETF_BENCHMARK.to_owned(),
        summary: summarize(&rows, &costs, hold_days),
        years: yearly(&rows, &costs),
        contributors: BacktestContributors {
            held_symbols: held_symbols as i64,
            best: best.into_iter().map(contributor).collect(),
            worst: worst.into_iter().map(contributor).collect(),
        },
        periods: rows.into_iter().zip(costs).map(period).collect(),
    })
    .into_response()
}

/// 個股貢獻名單的股票名稱。
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
