//! 年度股利合計列（`quarter = ''`）：由分期明細加總寫入，與找出殘留配息日期的合計列。

use anyhow::{Context, Result};

use crate::infra::database;

/// 合併並更新指定股票在指定發放年度的年度股利合計。
///
/// 合計列是明細的加總而非一次配發，日期一律為 `'-'`。衝突更新時必須連日期一起覆寫：
/// 該列可能是由既有的年度配息列原地轉生（股票從年配改成半年配時，原本 `quarter = ''`
/// 的明細列會與新的合計列撞上同一個年度層級唯一鍵），留著舊日期會讓合計被下游當成真實的配息事件。
pub(super) async fn upsert_annual_total_dividend(security_code: &str, year: i32) -> Result<()> {
    let sql = r#"
        INSERT INTO dividend(security_code,
               year,
               year_of_dividend,
               quarter,
               cash_dividend,
               stock_dividend,
               sum,
               "ex-dividend_date1",
               "ex-dividend_date2",
               payable_date1,
               payable_date2,
               created_time,
               updated_time,
               capital_reserve_cash_dividend,
               earnings_cash_dividend,
               capital_reserve_stock_dividend,
               earnings_stock_dividend,
               payout_ratio_cash,
               payout_ratio_stock,
               payout_ratio)
        SELECT security_code,
               $1,
               $2,
               '',
               sum(cash_dividend) as cash_dividend,
               sum(stock_dividend) as stock_dividend,
               sum(sum) as sum,
               '-',
               '-',
               '-',
               '-',
               now(),
               now(),
               0,
               0,
               0,
               0,
               0,
               0,
               0
               from dividend
        where security_code = $3 and year = $4 and quarter != ''
        group by security_code
        order by security_code
        ON CONFLICT (security_code, "year") WHERE quarter = '' DO UPDATE SET
            year_of_dividend = EXCLUDED.year_of_dividend,
            cash_dividend = EXCLUDED.cash_dividend,
            stock_dividend = EXCLUDED.stock_dividend,
            sum = EXCLUDED.sum,
            "ex-dividend_date1" = EXCLUDED."ex-dividend_date1",
            "ex-dividend_date2" = EXCLUDED."ex-dividend_date2",
            payable_date1 = EXCLUDED.payable_date1,
            payable_date2 = EXCLUDED.payable_date2,
            updated_time = EXCLUDED.updated_time;
    "#;
    sqlx::query(sql)
        .bind(year)
        .bind(year - 1)
        .bind(security_code)
        .bind(year)
        .execute(database::get_connection())
        .await
        .context("Failed to upsert annual total dividend")?;
    Ok(())
}

/// 找出帶有配息日期的年度合計列，回傳 (證券代號, 發放年度)。
pub(super) async fn fetch_stale_annual_total_dividends() -> Result<Vec<(String, i32)>> {
    let sql = r#"
        SELECT d.security_code, d.year
        FROM dividend AS d
        WHERE d.quarter = ''
            AND (
                d."ex-dividend_date1" <> '-'
                OR d."ex-dividend_date2" <> '-'
                OR d.payable_date1 <> '-'
                OR d.payable_date2 <> '-'
            )
            AND EXISTS (
                SELECT 1
                FROM dividend AS t
                WHERE t.security_code = d.security_code
                    AND t.year = d.year
                    AND t.quarter <> ''
            )
        ORDER BY d.security_code, d.year;
    "#;

    let rows = sqlx::query_as(sql)
        .fetch_all(database::get_connection())
        .await
        .context("Failed to fetch stale annual total dividends")?;
    Ok(rows)
}
