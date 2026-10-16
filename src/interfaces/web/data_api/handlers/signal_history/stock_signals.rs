//! `/stocks/{symbol}/signals` handler：個股出現過的技術訊號與每次訊號之後的報酬。

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Local, NaiveDate};

use super::rows::StockSignalRow;
use super::sql::STOCK_SIGNALS_SQL;
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, StockSignalItem, StockSignalsParams, StockSignalsResponse,
};
use crate::interfaces::web::data_api::handlers::{
    database_error, decimal_to_f64, ensure_stock_exists, error_response, parse_optional_date,
    signal_filter_error, signal_indicator_order, signal_kind_order,
};

/// 查詢個股在期間內出現過的技術訊號，以及每次訊號之後 5／20／60／120／240 個交易日的還原報酬。
///
/// # Errors
///
/// 日期格式錯誤、`days` 不在 1–730、`limit` 不在 1–200 或 `kind`／`indicator` 不在固定清單回 422；
/// 股票不存在回 404；驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。期間內沒有訊號回空清單。
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/signals", tag = "data-api", params(("symbol" = String, Path, description = "股票代號"), StockSignalsParams), responses((status = 200, body = StockSignalsResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn stock_signals(
    Path(symbol): Path<String>,
    Query(params): Query<StockSignalsParams>,
) -> Response {
    let to = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value.unwrap_or_else(|| Local::now().date_naive()),
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let days = params.days.unwrap_or(180);
    if !(1..=730).contains(&days) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "days 必須介於 1 至 730");
    }
    let limit = params.limit.unwrap_or(50);
    if !(1..=200).contains(&limit) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 200");
    }
    if let Some(response) = signal_filter_error(params.kind.as_deref(), params.indicator.as_deref())
    {
        return response;
    }
    if let Some(response) = ensure_stock_exists(&symbol).await {
        return response;
    }

    let from = to - chrono::Duration::days(i64::from(days) - 1);
    let rows: Vec<StockSignalRow> = match sqlx::query_as(STOCK_SIGNALS_SQL)
        .bind(&symbol)
        .bind(from)
        .bind(to)
        .bind(params.indicator.as_deref())
        .bind(params.kind.as_deref())
        .bind(i64::from(limit))
        .fetch_all(database::get_connection())
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };

    Json(build_stock_signals(symbol, from, to, rows)).into_response()
}

/// 組成個股訊號歷史；同一天的訊號依來源、種類的固定順序。
pub(super) fn build_stock_signals(
    symbol: String,
    from: NaiveDate,
    to: NaiveDate,
    mut rows: Vec<StockSignalRow>,
) -> StockSignalsResponse {
    let total = rows.first().map_or(0, |row| row.total);
    rows.sort_by(|a, b| {
        b.date
            .cmp(&a.date)
            .then_with(|| {
                signal_indicator_order(&a.indicator).cmp(&signal_indicator_order(&b.indicator))
            })
            .then_with(|| signal_kind_order(&a.kind).cmp(&signal_kind_order(&b.kind)))
    });
    StockSignalsResponse {
        stock_symbol: symbol,
        from: from.to_string(),
        to: to.to_string(),
        adjusted: true,
        total,
        items: rows
            .into_iter()
            .map(|row| StockSignalItem {
                date: row.date.to_string(),
                indicator: row.indicator,
                kind: row.kind,
                description: row.description,
                closing_price: decimal_to_f64(row.closing_price),
                change_percent: decimal_to_f64(row.change_range),
                return_5d: decimal_to_f64(row.return_5d),
                return_20d: decimal_to_f64(row.return_20d),
                return_60d: decimal_to_f64(row.return_60d),
                return_120d: decimal_to_f64(row.return_120d),
                return_240d: decimal_to_f64(row.return_240d),
            })
            .collect(),
    }
}
