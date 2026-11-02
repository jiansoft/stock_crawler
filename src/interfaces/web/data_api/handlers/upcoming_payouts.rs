//! `/portfolio/upcoming-payouts` handler：持股未來幾天的股利入帳（現金與股票股利的發放日）。
//!
//! 已除權息的股利取除權息當天寫入的已領股利明細（`dividend_record_detail_more`），所以除權息後才
//! 賣出、尚未發放的也列入；還沒除權息（或除權息當天尚未重算）的以目前未賣出持股推算並標
//! `estimated`。年度合計列與分期明細並存時只取分期，與已領股利計算相同。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Days, Local, NaiveDate};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::{database_error, error_response};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, UpcomingPayoutItem, UpcomingPayoutsParams, UpcomingPayoutsResponse,
};

/// 發放日落在期間內的股利，附已領明細或目前持股推算的股數與金額。
///
/// `$1` 會員（0 為全部）、`$2` 起日、`$3` 迄日、`$4` 今天（除權息日在今天以後才推算）。
pub(super) const PAYOUTS_SQL: &str = r#"
WITH due AS (
    SELECT d.serial, d.security_code, d.year, d.quarter, d.cash_dividend, d.stock_dividend,
           CASE WHEN d.payable_date1 ~ '^\d{4}-\d{2}-\d{2}$' THEN d.payable_date1::date END AS cash_payable,
           CASE WHEN d.payable_date2 ~ '^\d{4}-\d{2}-\d{2}$' THEN d.payable_date2::date END AS stock_payable,
           CASE WHEN d."ex-dividend_date1" ~ '^\d{4}-\d{2}-\d{2}$' THEN d."ex-dividend_date1"::date END AS cash_ex,
           CASE WHEN d."ex-dividend_date2" ~ '^\d{4}-\d{2}-\d{2}$' THEN d."ex-dividend_date2"::date END AS stock_ex
    FROM dividend AS d
    WHERE (
            (CASE WHEN d.payable_date1 ~ '^\d{4}-\d{2}-\d{2}$' THEN d.payable_date1::date END BETWEEN $2 AND $3
             AND d.cash_dividend > 0)
         OR (CASE WHEN d.payable_date2 ~ '^\d{4}-\d{2}-\d{2}$' THEN d.payable_date2::date END BETWEEN $2 AND $3
             AND d.stock_dividend > 0)
        )
        AND (
            d.quarter <> ''
            OR NOT EXISTS (
                SELECT 1 FROM dividend AS t
                WHERE t.security_code = d.security_code AND t.year = d.year AND t.quarter <> ''
            )
        )
), recorded AS (
    SELECT m.dividend_serial AS serial,
           sum(sod.share_quantity)::int8 AS shares,
           sum(m.cash) AS cash_amount,
           sum(m.stock) AS stock_shares
    FROM dividend_record_detail_more AS m
    JOIN stock_ownership_details AS sod ON sod.serial = m.stock_ownership_details_serial
    WHERE m.dividend_serial IN (SELECT serial FROM due)
        AND ($1 = 0 OR sod.member_id = $1)
    GROUP BY m.dividend_serial
), estimated AS (
    SELECT due.serial, sum(sod.share_quantity)::int8 AS shares
    FROM due
    JOIN stock_ownership_details AS sod ON sod.security_code = due.security_code
    WHERE sod.is_sold = false
        AND ($1 = 0 OR sod.member_id = $1)
        AND NOT EXISTS (SELECT 1 FROM recorded AS r WHERE r.serial = due.serial)
        AND (due.cash_ex >= $4 OR due.stock_ex >= $4)
    GROUP BY due.serial
)
SELECT due.security_code AS stock_symbol,
       COALESCE(s."Name", '') AS name,
       due.quarter,
       due.cash_dividend,
       due.stock_dividend,
       due.cash_payable,
       due.stock_payable,
       due.cash_ex,
       due.stock_ex,
       COALESCE(r.shares, e.shares, 0)::int8 AS shares,
       COALESCE(r.cash_amount, due.cash_dividend * e.shares, 0)::numeric AS cash_amount,
       COALESCE(r.stock_shares, floor(due.stock_dividend * e.shares / 10), 0)::numeric AS stock_shares,
       r.serial IS NULL AS estimated
FROM due
LEFT JOIN recorded AS r ON r.serial = due.serial
LEFT JOIN estimated AS e ON e.serial = due.serial
LEFT JOIN stocks AS s ON s.stock_symbol = due.security_code
WHERE r.serial IS NOT NULL OR e.serial IS NOT NULL
"#;

/// 一筆股利與持股的分配結果（現金與股票股利的發放日可能不同）。
#[derive(Debug, Clone, sqlx::FromRow)]
pub(super) struct PayoutRow {
    pub(super) stock_symbol: String,
    pub(super) name: String,
    pub(super) quarter: String,
    pub(super) cash_dividend: Decimal,
    pub(super) stock_dividend: Decimal,
    pub(super) cash_payable: Option<NaiveDate>,
    pub(super) stock_payable: Option<NaiveDate>,
    pub(super) cash_ex: Option<NaiveDate>,
    pub(super) stock_ex: Option<NaiveDate>,
    pub(super) shares: i64,
    pub(super) cash_amount: Decimal,
    pub(super) stock_shares: Decimal,
    pub(super) estimated: bool,
}

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

    let rows: Vec<PayoutRow> = match sqlx::query_as(PAYOUTS_SQL)
        .bind(member_id)
        .bind(today)
        .bind(to)
        .bind(today)
        .fetch_all(database::get_connection())
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    Json(build_response(member_id, today, to, &rows)).into_response()
}

/// 組出回應：每列依現金、股票股利各自的發放日拆成入帳，同一天的合為一筆；依發放日、代號排序。
pub(super) fn build_response(
    member_id: i64,
    from: NaiveDate,
    to: NaiveDate,
    rows: &[PayoutRow],
) -> UpcomingPayoutsResponse {
    let in_range = |date: Option<NaiveDate>| date.filter(|date| (from..=to).contains(date));
    let mut items = Vec::with_capacity(rows.len());
    let mut total = Decimal::ZERO;
    for row in rows {
        let cash = in_range(row.cash_payable).filter(|_| row.cash_amount > Decimal::ZERO);
        let stock_shares = row.stock_shares.floor().to_i64().unwrap_or_default();
        let stock = in_range(row.stock_payable).filter(|_| stock_shares > 0);
        if let Some(date) = cash {
            total += row.cash_amount;
            let mut item = payout_item(row, date, row.cash_ex.or(row.stock_ex));
            item.cash_dividend = to_f64(row.cash_dividend, 4);
            item.cash_amount = to_f64(row.cash_amount, 2);
            if stock == Some(date) {
                item.stock_dividend = to_f64(row.stock_dividend, 4);
                item.stock_shares = stock_shares;
            }
            items.push(item);
        }
        if let Some(date) = stock.filter(|date| cash != Some(*date)) {
            let mut item = payout_item(row, date, row.stock_ex.or(row.cash_ex));
            item.stock_dividend = to_f64(row.stock_dividend, 4);
            item.stock_shares = stock_shares;
            items.push(item);
        }
    }
    items.sort_by(|a, b| {
        (&a.payable_date, &a.stock_symbol, &a.quarter).cmp(&(
            &b.payable_date,
            &b.stock_symbol,
            &b.quarter,
        ))
    });
    UpcomingPayoutsResponse {
        member_id,
        from: from.to_string(),
        to: to.to_string(),
        total_cash_amount: to_f64(total, 2),
        items,
    }
}

/// 金額與股利都是 0 的入帳骨架，呼叫端再填這天入帳的部分。
fn payout_item(
    row: &PayoutRow,
    payable_date: NaiveDate,
    ex_date: Option<NaiveDate>,
) -> UpcomingPayoutItem {
    UpcomingPayoutItem {
        payable_date: payable_date.to_string(),
        stock_symbol: row.stock_symbol.clone(),
        name: row.name.clone(),
        quarter: row.quarter.clone(),
        ex_date: ex_date.map(|date| date.to_string()),
        shares: row.shares,
        cash_dividend: 0.0,
        stock_dividend: 0.0,
        cash_amount: 0.0,
        stock_shares: 0,
        estimated: row.estimated,
    }
}

/// 四捨五入到 `scale` 位小數後轉成 JSON 用的 `f64`。
fn to_f64(value: Decimal, scale: u32) -> f64 {
    value.round_dp(scale).to_f64().unwrap_or_default()
}

#[cfg(test)]
mod tests;
