//! 持股股利入帳查詢（[`PortfolioRepository::fetch_payout_allocations`]）。
//!
//! [`PortfolioRepository::fetch_payout_allocations`]: crate::domain::portfolio::repository::PortfolioRepository::fetch_payout_allocations

use anyhow::{Context, Result};
use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::domain::portfolio::payout::PayoutAllocation;
use crate::infra::database;

/// 發放日落在期間內的股利，依會員附已領明細或目前持股推算的股數與金額。
///
/// `$1` 會員（0 為全部）、`$2` 起日、`$3` 迄日、`$4` 今天（除權息日在今天以後才推算）。
/// 只要有任何會員的已領明細，就代表已除權息並重算過，其他會員不再推算。
const PAYOUTS_SQL: &str = r#"
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
           sod.member_id,
           sum(sod.share_quantity)::int8 AS shares,
           sum(m.cash) AS cash_amount,
           sum(m.stock) AS stock_shares
    FROM dividend_record_detail_more AS m
    JOIN stock_ownership_details AS sod ON sod.serial = m.stock_ownership_details_serial
    WHERE m.dividend_serial IN (SELECT serial FROM due)
        AND ($1 = 0 OR sod.member_id = $1)
    GROUP BY m.dividend_serial, sod.member_id
), estimated AS (
    SELECT due.serial, sod.member_id, sum(sod.share_quantity)::int8 AS shares
    FROM due
    JOIN stock_ownership_details AS sod ON sod.security_code = due.security_code
    WHERE sod.is_sold = false
        AND ($1 = 0 OR sod.member_id = $1)
        AND NOT EXISTS (
            SELECT 1 FROM dividend_record_detail_more AS m WHERE m.dividend_serial = due.serial
        )
        AND (due.cash_ex >= $4 OR due.stock_ex >= $4)
    GROUP BY due.serial, sod.member_id
), allocation AS (
    SELECT serial, member_id, shares, cash_amount, stock_shares, false AS estimated
    FROM recorded
    UNION ALL
    SELECT e.serial, e.member_id, e.shares,
           due.cash_dividend * e.shares,
           floor(due.stock_dividend * e.shares / 10),
           true
    FROM estimated AS e
    JOIN due ON due.serial = e.serial
)
SELECT a.member_id,
       due.security_code AS stock_symbol,
       COALESCE(s."Name", '') AS name,
       due.quarter,
       due.cash_dividend,
       due.stock_dividend,
       due.cash_payable,
       due.stock_payable,
       due.cash_ex,
       due.stock_ex,
       a.shares,
       COALESCE(a.cash_amount, 0)::numeric AS cash_amount,
       COALESCE(a.stock_shares, 0)::numeric AS stock_shares,
       a.estimated
FROM allocation AS a
JOIN due ON due.serial = a.serial
LEFT JOIN stocks AS s ON s.stock_symbol = due.security_code
"#;

/// [`PAYOUTS_SQL`] 的一列。
#[derive(Debug, sqlx::FromRow)]
struct PayoutRow {
    member_id: i64,
    stock_symbol: String,
    name: String,
    quarter: String,
    cash_dividend: Decimal,
    stock_dividend: Decimal,
    cash_payable: Option<NaiveDate>,
    stock_payable: Option<NaiveDate>,
    cash_ex: Option<NaiveDate>,
    stock_ex: Option<NaiveDate>,
    shares: i64,
    cash_amount: Decimal,
    stock_shares: Decimal,
    estimated: bool,
}

impl From<PayoutRow> for PayoutAllocation {
    fn from(row: PayoutRow) -> Self {
        Self {
            member_id: row.member_id,
            stock_symbol: row.stock_symbol,
            name: row.name,
            quarter: row.quarter,
            cash_dividend: row.cash_dividend,
            stock_dividend: row.stock_dividend,
            cash_payable: row.cash_payable,
            stock_payable: row.stock_payable,
            cash_ex: row.cash_ex,
            stock_ex: row.stock_ex,
            shares: row.shares,
            cash_amount: row.cash_amount,
            stock_shares: row.stock_shares,
            estimated: row.estimated,
        }
    }
}

/// 見 [`PortfolioRepository::fetch_payout_allocations`]。
///
/// [`PortfolioRepository::fetch_payout_allocations`]: crate::domain::portfolio::repository::PortfolioRepository::fetch_payout_allocations
pub(super) async fn fetch(
    member_id: i64,
    from: NaiveDate,
    to: NaiveDate,
    today: NaiveDate,
) -> Result<Vec<PayoutAllocation>> {
    let rows: Vec<PayoutRow> = sqlx::query_as(PAYOUTS_SQL)
        .bind(member_id)
        .bind(from)
        .bind(to)
        .bind(today)
        .fetch_all(database::get_connection())
        .await
        .with_context(|| {
            format!("查詢股利入帳失敗: member_id={member_id}, from={from}, to={to}")
        })?;
    Ok(rows.into_iter().map(PayoutAllocation::from).collect())
}
