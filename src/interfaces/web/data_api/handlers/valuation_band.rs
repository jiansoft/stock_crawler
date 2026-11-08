//! `/stocks/{symbol}/valuation-band` handler：本益比、股價淨值比在近幾年的相對位置。
//!
//! 每日倍數來自 `DailyQuotes`（本益比為交易所公布值、股價淨值比為收盤價 ÷ 當時
//! 每股淨值）；統計規則在 [`crate::domain::quote::valuation_band`]。

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rust_decimal::{Decimal, prelude::ToPrimitive};

use super::{ensure_stock_exists, error_response, repository_error};
use crate::domain::quote::valuation_band::{self, DailyRatio, ValuationHistoryRepository};
use crate::infra::database::repository::quote::PgQuoteRepository;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, ValuationBandParams, ValuationBandResponse,
};

/// 查詢單一股票本益比與股價淨值比在近幾年的歷史區間。
///
/// 期間以最新收盤日往回推 `years` 年；倍數 ≤ 0 的日子（虧損、未提供）不算樣本。
///
/// # Errors
///
/// `years` 不在 1–10 回 422、股票不存在回 404、驗證失敗回 401；資料庫查詢失敗時
/// 記錄內部錯誤並回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/valuation-band", tag = "data-api", params(("symbol" = String, Path, description = "股票代號"), ValuationBandParams), responses((status = 200, body = ValuationBandResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn stock_valuation_band(
    Path(symbol): Path<String>,
    Query(params): Query<ValuationBandParams>,
) -> Response {
    let years = params.years.unwrap_or(5);
    if !(1..=10).contains(&years) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "years 必須介於 1 至 10");
    }
    if let Some(response) = ensure_stock_exists(&symbol).await {
        return response;
    }

    let rows = match PgQuoteRepository::new()
        .fetch_daily_ratios(&symbol, years)
        .await
    {
        Ok(rows) => rows,
        Err(error) => return repository_error(error),
    };

    Json(build_response(symbol, years, &rows)).into_response()
}

/// 由新到舊的每日倍數組成回應；沒有日線時各欄位為 `null`。
fn build_response(symbol: String, years: u8, rows: &[DailyRatio]) -> ValuationBandResponse {
    let latest = rows.first();
    let closing_price = latest.map(|row| row.close);
    let band = |pick: fn(&DailyRatio) -> Decimal| {
        let history: Vec<Decimal> = rows.iter().map(pick).collect();
        valuation_band::summarize(&history, latest.map(pick), closing_price).map(Into::into)
    };
    ValuationBandResponse {
        data_as_of: latest.map(|row| row.date.to_string()),
        years,
        from: rows.last().map(|row| row.date.to_string()),
        closing_price: closing_price.and_then(|price| price.to_f64()),
        price_earning_ratio: band(|row| row.price_earning_ratio),
        price_to_book_ratio: band(|row| row.price_to_book_ratio),
        stock_symbol: symbol,
    }
}

#[cfg(test)]
mod tests;
