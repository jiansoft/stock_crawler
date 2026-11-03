//! `/portfolio/upcoming-dividends` handler：目前持股未來幾天已公告的除權息、預估入帳與歷年填息。
//!
//! 持股與股數沿用 `/portfolio/dividend-forecast` 的查詢（未賣出、依代號合併）；除權息與金額規則在
//! [`crate::app::calculation::upcoming_dividends`]，與週六每週市場摘要的「下週持股除權息」相同。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Days, Local, NaiveDate};
use rust_decimal::Decimal;

use super::dividend_fill::summary_dto;
use super::market_dividend_fill::bucket_dto;
use super::{database_error, error_response};
use crate::app::calculation::dividend_fill_hints::FillHints;
use crate::app::calculation::upcoming_dividends::{
    UpcomingDividend, total_shares, upcoming_dividends as upcoming,
};
use crate::infra::database;
use crate::infra::database::repository::dividend::PgDividendRepository;
use crate::infra::database::repository::portfolio::forecast::{HOLDINGS_SQL, HoldingRow};
use crate::interfaces::web::data_api::dto::{
    ErrorBody, UpcomingDividendItem, UpcomingDividendsParams, UpcomingDividendsResponse,
};

/// 持股未來幾天（含今天）已公告的除權息。
///
/// 每筆附預估現金入帳、配股股數與近 5 年填息（沒有自身紀錄時附同殖利率組基準）。只含已公告
/// 除權息日的事件，不推估；金額未扣二代健保補充保費與稅。
///
/// # Errors
///
/// `member_id` 為負數或 `days` 不在 1–60 回 422、驗證失敗回 401；查詢持股失敗時記錄內部錯誤並回
/// 不含 SQL 細節的 500。某天的除權息查詢失敗只記錄警告、少那一天。沒有持股時回 200 與空清單。
#[utoipa::path(get, path = "/api/v1/portfolio/upcoming-dividends", tag = "data-api", params(UpcomingDividendsParams), responses((status = 200, body = UpcomingDividendsResponse), (status = 401, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn upcoming_dividends(Query(params): Query<UpcomingDividendsParams>) -> Response {
    let member_id = params.member_id.unwrap_or(0);
    if member_id < 0 {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "member_id 不可為負數");
    }
    let days = params.days.unwrap_or(14);
    if !(1..=60).contains(&days) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "days 必須介於 1 至 60");
    }
    let today = Local::now().date_naive();
    let to = today + Days::new(u64::from(days) - 1);

    let holdings: Vec<HoldingRow> = match sqlx::query_as(HOLDINGS_SQL)
        .bind(member_id)
        .fetch_all(database::get_connection())
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    let shares = total_shares(
        holdings
            .into_iter()
            .map(|row| (row.stock_symbol, row.shares)),
    );
    let dividends = upcoming(&PgDividendRepository::new(), &shares, today, to).await;
    let hints = if dividends.is_empty() {
        None
    } else {
        let symbols: Vec<String> = dividends
            .iter()
            .map(|dividend| dividend.stock_symbol.clone())
            .collect();
        Some(FillHints::load(&symbols, today).await)
    };
    Json(build_response(
        member_id,
        today,
        to,
        &dividends,
        hints.as_ref(),
    ))
    .into_response()
}

/// 組出回應；`hints` 為 `None`（沒有事件）時不附填息。
fn build_response(
    member_id: i64,
    from: NaiveDate,
    to: NaiveDate,
    dividends: &[UpcomingDividend],
    hints: Option<&FillHints>,
) -> UpcomingDividendsResponse {
    let total: Decimal = dividends.iter().map(UpcomingDividend::cash_amount).sum();
    let items = dividends
        .iter()
        .map(|dividend| {
            let mut item = UpcomingDividendItem::from(dividend);
            if let Some(hints) = hints {
                attach_fill_hints(&mut item, dividend, hints);
            }
            item
        })
        .collect();
    UpcomingDividendsResponse {
        member_id,
        from: from.to_string(),
        to: to.to_string(),
        total_cash_amount: rust_decimal::prelude::ToPrimitive::to_f64(&total.round_dp(2))
            .unwrap_or_default(),
        items,
    }
}

/// 附該股近 5 年填息；沒有自身紀錄時以每股現金股利 ÷ 最新收盤附同殖利率組基準。
fn attach_fill_hints(
    item: &mut UpcomingDividendItem,
    dividend: &UpcomingDividend,
    hints: &FillHints,
) {
    if let Some(history) = hints.history(&dividend.stock_symbol) {
        item.fill_history = Some(summary_dto(history));
        return;
    }
    item.fill_peer = hints
        .peer(dividend.cash_dividend, dividend.closing_price)
        .map(bucket_dto);
}

#[cfg(test)]
mod tests;
