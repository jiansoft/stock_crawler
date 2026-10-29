//! `/market/forward-returns` handler：大盤（同市場全部股票等權）各天期未來報酬的平均與上漲比例，
//! 整段期間、依年度與依月份（季節性）。
//!
//! 讀預先彙總的 `daily_market_forward_return`，與訊號績效、法人連買績效的大盤基準共用
//! [`fetch_benchmark`]；基準日落在期間內才計入。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Local, Months, NaiveDate};

use super::signal_history::{benchmark_output, fetch_benchmark, fetch_benchmark_by_month};
use super::{database_error, error_response, market_id_for_stats, parse_optional_date};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, MarketForwardReturnsParams, MarketForwardReturnsResponse,
};

/// 期間內最後一個有 5 日報酬的基準日。
const DATA_AS_OF_SQL: &str = r#"
SELECT max(date) FROM daily_market_forward_return
WHERE market_id = $1 AND date BETWEEN $2 AND $3 AND n5 > 0
"#;

/// 查詢大盤未來報酬。
///
/// # Errors
///
/// 參數格式錯誤、`years` 超出 1–20 或 `market` 不合法回 422；期間內沒有任何 5 日報酬回 404；驗證失敗回
/// 401；資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/forward-returns", tag = "data-api", params(MarketForwardReturnsParams), responses((status = 200, body = MarketForwardReturnsResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn market_forward_returns(
    Query(params): Query<MarketForwardReturnsParams>,
) -> Response {
    let date = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let years = params.years.unwrap_or(3);
    if !(1..=20).contains(&years) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "years 必須介於 1 至 20");
    }
    let market = params.market.as_deref().unwrap_or("all");
    let Some(market_id) = market_id_for_stats(market) else {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "market 必須為 all、twse 或 tpex",
        );
    };
    let to = date.unwrap_or_else(|| Local::now().date_naive());
    let from = to
        .checked_sub_months(Months::new(u32::from(years) * 12))
        .unwrap_or(NaiveDate::MIN);

    let data_as_of: Option<NaiveDate> = match sqlx::query_scalar(DATA_AS_OF_SQL)
        .bind(market_id)
        .bind(from)
        .bind(to)
        .fetch_one(database::get_connection())
        .await
    {
        Ok(value) => value,
        Err(error) => return database_error(error),
    };
    let Some(data_as_of) = data_as_of else {
        return error_response(StatusCode::NOT_FOUND, "期間內沒有未來報酬資料");
    };
    let benchmark = match fetch_benchmark(from, to, market_id, true).await {
        Ok(benchmark) => benchmark,
        Err(error) => return database_error(error),
    };
    let by_month = match fetch_benchmark_by_month(from, to, market_id).await {
        Ok(by_month) => by_month,
        Err(error) => return database_error(error),
    };
    let (benchmark, by_year) = benchmark_output(&benchmark);
    Json(MarketForwardReturnsResponse {
        from: from.to_string(),
        to: to.to_string(),
        years,
        market: market.to_owned(),
        data_as_of: data_as_of.to_string(),
        benchmark,
        by_year: by_year.unwrap_or_default(),
        by_month,
    })
    .into_response()
}
