//! `/market/institutional-streak/performance` handler：法人連續買賣超之後的報酬統計。
//!
//! 事件＝某檔股票的法人（`chip_daily`）連續買超（或賣超）剛好滿 N 天的那一天；之後延續的天數
//! 不重複計算。連續天數只看有籌碼資料的日子（與 `/market/institutional-flow` 相同，淨額為 0 會中斷）。
//! 事件之後的報酬、大盤基準與統計方式沿用 [`super::signal_history`] 的訊號績效。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;

use super::institutional_flow::INVESTORS;
use super::signal_history::{
    BENCHMARK_SQL, BenchmarkRow, PerformanceRow, benchmark_stats, horizon_stats,
    performance_aggregate_sql, resolve_period,
};
use super::{
    database_error, error_response, industry_error, market_id_for_stocks, parse_optional_date,
};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, InstitutionalStreakPerformanceParams, InstitutionalStreakPerformanceResponse,
};

/// `$1`（含）之前最近一個有籌碼資料的交易日；`$1` 為 `NULL` 時取最新。
const LATEST_CHIP_DATE_SQL: &str = r#"
SELECT MAX("date") FROM chip_daily WHERE $1::date IS NULL OR "date" <= $1
"#;

/// 期間內連續買賣超滿 N 天的事件與其後的報酬統計。
///
/// `$1`／`$2` 起訖、`$3` 市場 id（0 為上市櫃全部）、`$4` 法人別、`$5` 方向（1 買超、-1 賣超）、
/// `$6` 連續天數 N、`$7` 產業名稱。往前多讀 60 個日曆天，讓期間開頭的事件也能數到 20 天。
/// 連續段以「依日期的列號 − 依方向分組的列號」切出（gaps and islands），段內第 N 天即為事件。
const STREAK_PERFORMANCE_SQL: &str = concat!(
    r#"
WITH net AS (
    SELECT c.stock_symbol, c."date",
           CASE $4
               WHEN 'foreign' THEN c.foreign_net
               WHEN 'trust' THEN c.trust_net
               WHEN 'dealer' THEN c.dealer_net
               ELSE c.foreign_net + c.trust_net + c.dealer_net
           END AS net
    FROM chip_daily c
    JOIN stocks s ON s.stock_symbol = c.stock_symbol
    LEFT JOIN stock_industry si ON si.stock_industry_id = s.stock_industry_id
    WHERE c."date" BETWEEN $1::date - 60 AND $2
      AND s.stock_exchange_market_id IN (2, 4)
      AND ($3 = 0 OR s.stock_exchange_market_id = $3)
      AND ($7::text IS NULL OR si.name = $7)
),
runs AS (
    SELECT stock_symbol, "date", SIGN(net)::int AS sgn,
           ROW_NUMBER() OVER (PARTITION BY stock_symbol ORDER BY "date")
               - ROW_NUMBER() OVER (PARTITION BY stock_symbol, SIGN(net) ORDER BY "date") AS grp
    FROM net
    WHERE net IS NOT NULL
),
counted AS (
    SELECT stock_symbol, "date", sgn,
           ROW_NUMBER() OVER (PARTITION BY stock_symbol, sgn, grp ORDER BY "date") AS len
    FROM runs
),
sig AS (
    SELECT "date" AS date, stock_symbol, 'institutional_streak'::text AS indicator, ''::text AS kind
    FROM counted
    WHERE sgn = $5 AND len = $6 AND "date" BETWEEN $1 AND $2
),
"#,
    performance_aggregate_sql!()
);

/// 統計期間內法人連續買超（或賣超）滿 N 天之後 5／20／60／120／240 個交易日的表現。
///
/// 報酬為事件當天收盤到之後第 N 個交易日收盤的還原報酬；尚未滿天期的事件不列入該天期樣本。
/// 超額報酬以同一天同市場全部股票的平均報酬為基準。籌碼資料從 2025-10 起，長天期樣本較少。
///
/// # Errors
///
/// 日期格式錯誤、起日晚於迄日、期間超過 1,096 天、`streak` 不在 1–20、`investor`／`side`／`market`
/// 不合法或產業名稱不存在回 422；迄日之前沒有任何籌碼資料回 404；驗證失敗回 401；資料庫查詢失敗回
/// 不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/institutional-streak/performance", tag = "data-api", params(InstitutionalStreakPerformanceParams), responses((status = 200, body = InstitutionalStreakPerformanceResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn institutional_streak_performance(
    Query(params): Query<InstitutionalStreakPerformanceParams>,
) -> Response {
    let (from, to) = match (
        parse_optional_date(params.from.as_deref()),
        parse_optional_date(params.to.as_deref()),
    ) {
        (Ok(from), Ok(to)) => (from, to),
        (Err(message), _) | (_, Err(message)) => {
            return error_response(StatusCode::UNPROCESSABLE_ENTITY, message);
        }
    };
    let investor = params.investor.as_deref().unwrap_or("foreign");
    if !INVESTORS.contains(&investor) {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "investor 必須為 foreign、trust、dealer 或 total",
        );
    }
    let side = params.side.as_deref().unwrap_or("buy");
    let sign: i32 = match side {
        "buy" => 1,
        "sell" => -1,
        _ => return error_response(StatusCode::UNPROCESSABLE_ENTITY, "side 必須為 buy 或 sell"),
    };
    let streak = params.streak.unwrap_or(3);
    if !(1..=20).contains(&streak) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "streak 必須介於 1 至 20");
    }
    let market = params.market.as_deref().unwrap_or("all");
    let Some(market_id) = market_id_for_stocks(market) else {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "market 必須為 all、twse 或 tpex",
        );
    };
    if let Some(response) = industry_error(params.industry.as_deref()).await {
        return response;
    }

    let pool = database::get_connection();
    let latest: Option<NaiveDate> = match sqlx::query_scalar(LATEST_CHIP_DATE_SQL)
        .bind(to)
        .fetch_one(pool)
        .await
    {
        Ok(value) => value,
        Err(error) => return database_error(error),
    };
    let Some(to) = latest else {
        return error_response(StatusCode::NOT_FOUND, "查無籌碼資料");
    };
    let from = match resolve_period(from, to) {
        Ok(from) => from,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };

    let rows: Vec<PerformanceRow> = match sqlx::query_as(STREAK_PERFORMANCE_SQL)
        .bind(from)
        .bind(to)
        .bind(market_id)
        .bind(investor)
        .bind(sign)
        .bind(i64::from(streak))
        .bind(params.industry.as_deref())
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    let benchmark: BenchmarkRow = match sqlx::query_as(BENCHMARK_SQL)
        .bind(from)
        .bind(to)
        .bind(market_id)
        .fetch_one(pool)
        .await
    {
        Ok(row) => row,
        Err(error) => return database_error(error),
    };

    let row = rows.into_iter().next().unwrap_or_default();
    Json(InstitutionalStreakPerformanceResponse {
        from: from.to_string(),
        to: to.to_string(),
        market: market.to_owned(),
        investor: investor.to_owned(),
        side: side.to_owned(),
        streak,
        industry: params.industry.clone(),
        adjusted: true,
        occurrences: row.occurrences,
        horizons: horizon_stats(&row),
        benchmark: benchmark_stats(&benchmark),
    })
    .into_response()
}
