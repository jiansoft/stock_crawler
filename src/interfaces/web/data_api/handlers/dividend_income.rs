//! `/portfolio/dividend-income` handler：持股某一年 1～12 月的股利入帳，附去年同月與同期對照。
//!
//! 查詢沿用 [`crate::domain::portfolio::payout`] 與 `PgPortfolioRepository`（與
//! `/portfolio/upcoming-payouts`、入帳通知相同）；這裡只驗證參數、依月份加總並轉成 JSON。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Datelike, Local, Months, NaiveDate};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::upcoming_payouts::payout_item;
use super::{error_response, repository_error};
use crate::domain::portfolio::payout::{self, DividendPayout, MonthlyIncome};
use crate::domain::portfolio::repository::PortfolioRepository;
use crate::infra::database::repository::portfolio::PgPortfolioRepository;
use crate::interfaces::web::data_api::dto::{
    DividendIncomeMonth, DividendIncomeParams, DividendIncomeResponse, ErrorBody,
};

/// 持股某一年 1～12 月的股利入帳（依發放日）。
///
/// 已除權息的金額取已領股利明細（除權息後才賣出的也算），尚未除權息的以目前持股推算
/// （`estimated_cash_amount`）；未來月份只含已公告發放日的股利。金額未扣二代健保補充保費與稅。
/// 帶 `include_items=true` 時另回今年每一筆入帳（`items`），供月曆點月份看明細。
///
/// # Errors
///
/// `member_id` 為負數或 `year` 不在 2000 到明年回 422、驗證失敗回 401；查詢失敗時記錄內部錯誤並回
/// 不含 SQL 細節的 500。沒有入帳時回 200，12 個月都是 0。
#[utoipa::path(get, path = "/api/v1/portfolio/dividend-income", tag = "data-api", params(DividendIncomeParams), responses((status = 200, body = DividendIncomeResponse), (status = 401, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn dividend_income(Query(params): Query<DividendIncomeParams>) -> Response {
    let member_id = params.member_id.unwrap_or(0);
    if member_id < 0 {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "member_id 不可為負數");
    }
    let today = Local::now().date_naive();
    let year = params.year.unwrap_or(today.year());
    let Some((from, to)) = year_range(year, today) else {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "year 必須介於 2000 至明年",
        );
    };
    let (last_from, last_to) = (from - Months::new(12), to - Months::new(12));

    let repo = PgPortfolioRepository::new();
    let (this_year, last_year) = match tokio::try_join!(
        payouts_between(&repo, member_id, from, to, today),
        payouts_between(&repo, member_id, last_from, last_to, today),
    ) {
        Ok(payouts) => payouts,
        Err(error) => return repository_error(error),
    };
    let include_items = params.include_items.unwrap_or(false);
    Json(build_response(
        member_id,
        year,
        today,
        &this_year,
        &last_year,
        include_items,
    ))
    .into_response()
}

/// `year` 的 1 月 1 日與 12 月 31 日；年份不在 2000 到 `today` 的明年時回 `None`。
fn year_range(year: i32, today: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
    if !(2000..=today.year() + 1).contains(&year) {
        return None;
    }
    Some((
        NaiveDate::from_ymd_opt(year, 1, 1)?,
        NaiveDate::from_ymd_opt(year, 12, 31)?,
    ))
}

async fn payouts_between(
    repo: &impl PortfolioRepository,
    member_id: i64,
    from: NaiveDate,
    to: NaiveDate,
    today: NaiveDate,
) -> anyhow::Result<Vec<DividendPayout>> {
    let allocations = repo
        .fetch_payout_allocations(member_id, from, to, today)
        .await?;
    Ok(payout::merge_members(payout::split_by_payable_date(
        &allocations,
        from,
        to,
    )))
}

/// 組出回應：今年與去年的入帳各自依月份加總，去年同期取到去年的 `as_of` 同日（年度已過完時為全年）；
/// `include_items` 時附上今年每一筆入帳。
pub(super) fn build_response(
    member_id: i64,
    year: i32,
    as_of: NaiveDate,
    this_year: &[DividendPayout],
    last_year: &[DividendPayout],
    include_items: bool,
) -> DividendIncomeResponse {
    let months = payout::monthly_income(this_year, year, as_of);
    let last_months = payout::monthly_income(last_year, year - 1, as_of);
    let same_period_end = as_of - Months::new(12);
    let last_year_same_period: Decimal = last_year
        .iter()
        .filter(|item| item.payable_date <= same_period_end)
        .map(|item| item.cash_amount)
        .sum();
    let sum = |field: fn(&MonthlyIncome) -> Decimal| months.iter().map(field).sum::<Decimal>();

    DividendIncomeResponse {
        member_id,
        year,
        as_of: as_of.to_string(),
        total_cash_amount: to_f64(sum(|m| m.cash_amount)),
        received_cash_amount: to_f64(sum(|m| m.received_cash_amount)),
        estimated_cash_amount: to_f64(sum(|m| m.estimated_cash_amount)),
        total_stock_shares: months.iter().map(|m| m.stock_shares).sum(),
        last_year_cash_amount: to_f64(last_months.iter().map(|m| m.cash_amount).sum()),
        last_year_same_period_cash_amount: to_f64(last_year_same_period),
        months: months
            .iter()
            .zip(&last_months)
            .map(|(month, last)| DividendIncomeMonth {
                month: month.month,
                cash_amount: to_f64(month.cash_amount),
                received_cash_amount: to_f64(month.received_cash_amount),
                estimated_cash_amount: to_f64(month.estimated_cash_amount),
                stock_shares: month.stock_shares,
                payouts: month.payouts,
                last_year_cash_amount: to_f64(last.cash_amount),
            })
            .collect(),
        items: include_items.then(|| this_year.iter().map(payout_item).collect()),
    }
}

/// 金額四捨五入到 2 位小數後轉成 JSON 用的 `f64`。
fn to_f64(value: Decimal) -> f64 {
    value.round_dp(2).to_f64().unwrap_or_default()
}

#[cfg(test)]
mod tests;
