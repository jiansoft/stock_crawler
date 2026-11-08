//! `/portfolio/dividend-forecast` handler：以目前持股預估未來幾個月的現金股利入帳。
//!
//! 持股、配息與已除息的已領明細由
//! [`crate::infra::database::repository::portfolio::forecast::load_inputs`] 讀取（每月快照排程共用），
//! 預估規則在 [`crate::domain::portfolio::dividend_forecast`]。

use std::collections::HashMap;

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Local, NaiveDate};
use rust_decimal::Decimal;

use super::dividend_fill::summary_dto;
use super::market_dividend_fill::bucket_dto;
use super::{error_response, repository_error};
use crate::app::calculation::dividend_fill_hints::FillHints;
use crate::domain::portfolio::dividend_forecast;
use crate::infra::database::repository::portfolio::forecast::{self, ForecastInputs};
use crate::interfaces::web::data_api::dto::{
    DividendForecastItem, DividendForecastParams, DividendForecastResponse, ErrorBody,
};

/// 以目前持股預估未來幾個月每月的現金股利入帳。
///
/// 已公告的配息直接列入；過去 12 個月的配息往後推一年估算（季配、月配金額用最近一期，
/// 月配另依配息間隔補上去年同期沒有的月份）。只含現金股利，
/// 不扣二代健保補充保費與稅；已除息未發放的用除息日持股，其餘假設目前持股持有到除息日。
///
/// # Errors
///
/// `member_id` 為負數或 `months` 不在 1–24 回 422、驗證失敗回 401；資料庫查詢失敗時
/// 記錄內部錯誤並回不含 SQL 細節的 500。沒有持股時回 200 與 0。
#[utoipa::path(get, path = "/api/v1/portfolio/dividend-forecast", tag = "data-api", params(DividendForecastParams), responses((status = 200, body = DividendForecastResponse), (status = 401, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn dividend_forecast(Query(params): Query<DividendForecastParams>) -> Response {
    let member_id = params.member_id.unwrap_or(0);
    if member_id < 0 {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "member_id 不可為負數");
    }
    let months = params.months.unwrap_or(12);
    if !(1..=24).contains(&months) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "months 必須介於 1 至 24");
    }
    let today = Local::now().date_naive();
    let inputs = match forecast::load_inputs(member_id, today, u32::from(months)).await {
        Ok(inputs) => inputs,
        Err(error) => return repository_error(error),
    };

    let symbols: Vec<String> = inputs
        .holdings
        .iter()
        .map(|holding| holding.stock_symbol.clone())
        .collect();
    let closing_prices: HashMap<String, Decimal> = inputs
        .holdings
        .iter()
        .filter_map(|holding| Some((holding.stock_symbol.clone(), holding.closing_price?)))
        .collect();
    let mut response = build_response(member_id, &inputs, today, months);
    let hints = FillHints::load(&symbols, today).await;
    attach_fill_hints(&mut response.items, &hints, &closing_prices);
    Json(response).into_response()
}

/// 每筆預估附該股近 5 年填息；沒有自身紀錄時以每股現金股利 ÷ 最新收盤附同殖利率組基準。
fn attach_fill_hints(
    items: &mut [DividendForecastItem],
    hints: &FillHints,
    closing_prices: &HashMap<String, Decimal>,
) {
    for item in items {
        if let Some(history) = hints.history(&item.stock_symbol) {
            item.fill_history = Some(summary_dto(history));
            continue;
        }
        let cash = Decimal::try_from(item.cash_per_share).unwrap_or_default();
        item.fill_peer = closing_prices
            .get(&item.stock_symbol)
            .and_then(|price| hints.peer(cash, *price))
            .map(bucket_dto);
    }
}

/// 由查詢結果組出回應。
fn build_response(
    member_id: i64,
    inputs: &ForecastInputs,
    today: NaiveDate,
    months: u8,
) -> DividendForecastResponse {
    let result = dividend_forecast::forecast(
        &inputs.holdings,
        &inputs.events,
        &inputs.entitlements,
        today,
        u32::from(months),
    );
    DividendForecastResponse::new(member_id, result)
}

#[cfg(test)]
mod tests;
