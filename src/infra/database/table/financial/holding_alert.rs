//! 持股通知用的查詢。
//!
//! 這裡的查詢只服務 Telegram 的月營收與財報通知，且一律只看**目前持有**的股票
//! （`stock_ownership_details.is_sold = false`）。全市場每個月有數百檔營收年增率超過門檻，
//! 全推等於把通知變成雜訊。
//!
//! 兩個查詢都是唯讀，不會寫入任何資料。

use anyhow::Result;
use rust_decimal::Decimal;
use sqlx::{Row, postgres::PgRow};

use crate::domain::financial::entity::{HoldingFinancialAlert, HoldingRevenueAlert};
use crate::infra::database;

// 兩個查詢都以 `security_code IN (SELECT ... WHERE is_sold = false)` 限定持股。
// 這段條件刻意寫在各自的字面 SQL 裡而不是用 format! 組出來：
// sqlx 0.9 只接受 &'static str（避免動態 SQL 注入），而這裡本來就沒有動態成分。

/// 取得指定月份中，年增率絕對值達 `yoy_threshold` 的持股月營收。
///
/// `date` 為營收月份，格式 yyyyMM（與 `"Revenue"."Date"` 一致）。
/// 依年增率絕對值由大到小排序，最極端的變化排在最前面。
///
/// # Errors
/// 當 SQL 執行失敗時回傳錯誤。
pub async fn fetch_holding_revenue_alerts(
    date: i64,
    yoy_threshold: Decimal,
) -> Result<Vec<HoldingRevenueAlert>> {
    let alerts = sqlx::query(
        r#"
SELECT
    r."stock_symbol",
    s."Name" AS stock_name,
    r."Date",
    r."Monthly",
    r."ComparedWithLastMonth",
    r."ComparedWithLastYearSameMonth"
FROM "Revenue" AS r
INNER JOIN stocks AS s ON s.stock_symbol = r."stock_symbol"
WHERE r."Date" = $1
    AND r."stock_symbol" IN (
        SELECT security_code FROM stock_ownership_details WHERE is_sold = false
    )
    AND ABS(r."ComparedWithLastYearSameMonth") >= $2
ORDER BY ABS(r."ComparedWithLastYearSameMonth") DESC, r."stock_symbol"
        "#,
    )
    .bind(date)
    .bind(yoy_threshold)
    .try_map(|row: PgRow| {
        Ok(HoldingRevenueAlert {
            stock_symbol: row.try_get("stock_symbol")?,
            stock_name: row.try_get("stock_name")?,
            date: row.try_get("Date")?,
            monthly: row.try_get("Monthly")?,
            compared_with_last_month: row.try_get("ComparedWithLastMonth")?,
            compared_with_last_year_same_month: row.try_get("ComparedWithLastYearSameMonth")?,
        })
    })
    .fetch_all(database::get_connection())
    .await?;

    Ok(alerts)
}

/// 取得指定年度、季度的持股季報，並帶出去年同季的 EPS 供比較。
///
/// 去年同季以 LEFT JOIN 取得：新上市或財報尚未回補完整時查不到，
/// 此時 `last_year_earnings_per_share` 為 `None`，通知會略過比較而不是顯示 0。
///
/// # Errors
/// 當 SQL 執行失敗時回傳錯誤。
pub async fn fetch_holding_financial_alerts(
    year: i32,
    quarter: &str,
) -> Result<Vec<HoldingFinancialAlert>> {
    let alerts = sqlx::query(
        r#"
SELECT
    fs.security_code AS stock_symbol,
    s."Name" AS stock_name,
    fs.year,
    fs.quarter,
    fs.earnings_per_share,
    fs.return_on_equity,
    fs.gross_profit,
    prev.earnings_per_share AS last_year_earnings_per_share
FROM financial_statement AS fs
INNER JOIN stocks AS s ON s.stock_symbol = fs.security_code
LEFT JOIN financial_statement AS prev
    ON prev.security_code = fs.security_code
    AND prev.year = fs.year - 1
    AND prev.quarter = fs.quarter
WHERE fs.year = $1
    AND fs.quarter = $2
    AND fs.security_code IN (
        SELECT security_code FROM stock_ownership_details WHERE is_sold = false
    )
ORDER BY fs.security_code
        "#,
    )
    .bind(year)
    .bind(quarter)
    .try_map(|row: PgRow| {
        Ok(HoldingFinancialAlert {
            stock_symbol: row.try_get("stock_symbol")?,
            stock_name: row.try_get("stock_name")?,
            year: row.try_get("year")?,
            quarter: row.try_get("quarter")?,
            earnings_per_share: row.try_get("earnings_per_share")?,
            return_on_equity: row.try_get("return_on_equity")?,
            gross_profit: row.try_get("gross_profit")?,
            last_year_earnings_per_share: row.try_get("last_year_earnings_per_share")?,
        })
    })
    .fetch_all(database::get_connection())
    .await?;

    Ok(alerts)
}
