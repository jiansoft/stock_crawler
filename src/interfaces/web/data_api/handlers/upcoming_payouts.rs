//! `/portfolio/upcoming-payouts` handler：持股未來幾天的股利入帳（現金與股票股利的發放日）。
//!
//! 查詢與依發放日拆分在 [`crate::domain::portfolio::payout`] 與 `PgPortfolioRepository`
//! （入帳通知共用）；這裡只驗證參數、合併會員並轉成 JSON。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Days, Local, NaiveDate};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::{error_response, repository_error};
use crate::domain::portfolio::payout::{self, DividendPayout};
use crate::domain::portfolio::repository::PortfolioRepository;
use crate::infra::database::repository::portfolio::PgPortfolioRepository;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, UpcomingPayoutItem, UpcomingPayoutsParams, UpcomingPayoutsResponse,
};

/// 持股未來幾天（含今天）的股利入帳。
///
/// 已除權息的金額取已領股利明細，尚未除權息的以目前持股推算（`estimated`）；金額未扣二代健保
/// 補充保費與稅。
///
/// # Errors
///
/// `member_id` 為負數或 `days` 不在 1–90 回 422、驗證失敗回 401；查詢失敗時記錄內部錯誤並回不含
/// SQL 細節的 500。沒有入帳時回 200 與空清單。
#[utoipa::path(get, path = "/api/v1/portfolio/upcoming-payouts", tag = "data-api", params(UpcomingPayoutsParams), responses((status = 200, body = UpcomingPayoutsResponse), (status = 401, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn upcoming_payouts(Query(params): Query<UpcomingPayoutsParams>) -> Response {
    let member_id = params.member_id.unwrap_or(0);
    if member_id < 0 {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "member_id 不可為負數");
    }
    let days = params.days.unwrap_or(30);
    if !(1..=90).contains(&days) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "days 必須介於 1 至 90");
    }
    let today = Local::now().date_naive();
    let to = today + Days::new(u64::from(days) - 1);

    let allocations = match PgPortfolioRepository::new()
        .fetch_payout_allocations(member_id, today, to, today)
        .await
    {
        Ok(allocations) => allocations,
        Err(error) => return repository_error(error),
    };
    let payouts = payout::merge_members(payout::split_by_payable_date(&allocations, today, to));
    Json(build_response(member_id, today, to, &payouts)).into_response()
}

/// 組出回應：入帳已依發放日、代號排序並合併會員，金額轉成 JSON 用的小數。
pub(super) fn build_response(
    member_id: i64,
    from: NaiveDate,
    to: NaiveDate,
    payouts: &[DividendPayout],
) -> UpcomingPayoutsResponse {
    UpcomingPayoutsResponse {
        member_id,
        from: from.to_string(),
        to: to.to_string(),
        total_cash_amount: to_f64(payout::total_cash(payouts), 2),
        items: payouts.iter().map(payout_item).collect(),
    }
}

fn payout_item(payout: &DividendPayout) -> UpcomingPayoutItem {
    UpcomingPayoutItem {
        payable_date: payout.payable_date.to_string(),
        stock_symbol: payout.stock_symbol.clone(),
        name: payout.name.clone(),
        quarter: payout.quarter.clone(),
        ex_date: payout.ex_date.map(|date| date.to_string()),
        shares: payout.shares,
        cash_dividend: to_f64(payout.cash_dividend, 4),
        stock_dividend: to_f64(payout.stock_dividend, 4),
        cash_amount: to_f64(payout.cash_amount, 2),
        stock_shares: payout.stock_shares,
        estimated: payout.estimated,
    }
}

/// 四捨五入到 `scale` 位小數後轉成 JSON 用的 `f64`。
fn to_f64(value: Decimal, scale: u32) -> f64 {
    value.round_dp(scale).to_f64().unwrap_or_default()
}

#[cfg(test)]
mod tests;
