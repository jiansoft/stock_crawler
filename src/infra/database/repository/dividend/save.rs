//! 股利的儲存：年度層級列與分期明細的 upsert，以及同一次配息的所屬年度更正。

use anyhow::{Context, Result};
use chrono::Local;

use crate::domain::dividend::entity::Dividend;
use crate::infra::database;
use crate::infra::database::table::dividend::mutation;

/// 組出 [`save`] 的 upsert SQL；`$target` 為衝突目標。
macro_rules! save_sql {
    ($target:expr) => {
        concat!(
            r#"
            INSERT INTO dividend (
                security_code, "year", year_of_dividend, quarter,
                cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2",
                payable_date1, payable_date2, created_time, updated_time, capital_reserve_cash_dividend,
                earnings_cash_dividend, capital_reserve_stock_dividend, earnings_stock_dividend,
                payout_ratio_cash, payout_ratio_stock, payout_ratio)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20)
            ON CONFLICT "#,
            $target,
            r#" DO UPDATE SET
                year_of_dividend = EXCLUDED.year_of_dividend,
                cash_dividend = EXCLUDED.cash_dividend,
                stock_dividend = EXCLUDED.stock_dividend,
                "sum" = EXCLUDED."sum",
                "ex-dividend_date1" = EXCLUDED."ex-dividend_date1",
                "ex-dividend_date2" = EXCLUDED."ex-dividend_date2",
                payable_date1 = EXCLUDED.payable_date1,
                payable_date2 = EXCLUDED.payable_date2,
                updated_time = EXCLUDED.updated_time,
                capital_reserve_cash_dividend = EXCLUDED.capital_reserve_cash_dividend,
                earnings_cash_dividend = EXCLUDED.earnings_cash_dividend,
                capital_reserve_stock_dividend = EXCLUDED.capital_reserve_stock_dividend,
                earnings_stock_dividend = EXCLUDED.earnings_stock_dividend,
                -- 盈餘分配率只有 Goodinfo 提供，Yahoo 一律送 0。
                -- 直接覆蓋會讓每次股利採集都洗掉已回補的分配率，所以來源是 0 時保留既有值。
                payout_ratio_cash = CASE WHEN EXCLUDED.payout_ratio_cash = 0
                    THEN dividend.payout_ratio_cash ELSE EXCLUDED.payout_ratio_cash END,
                payout_ratio_stock = CASE WHEN EXCLUDED.payout_ratio_stock = 0
                    THEN dividend.payout_ratio_stock ELSE EXCLUDED.payout_ratio_stock END,
                payout_ratio = CASE WHEN EXCLUDED.payout_ratio = 0
                    THEN dividend.payout_ratio ELSE EXCLUDED.payout_ratio END;
        "#
        )
    };
}

/// 年度層級列的 upsert：每個發放年度一列。
const SAVE_ANNUAL_LEVEL_SQL: &str = save_sql!(mutation::annual_level_conflict_target!());
/// 分期明細的 upsert：以主鍵為衝突目標。
const SAVE_DETAIL_SQL: &str = save_sql!(mutation::detail_conflict_target!());

/// 同一發放年度、同期別、除權息日相同但所屬年度不同的列，視為同一次配息，改成新的所屬年度。
///
/// 參數：`$1` 代號、`$2` 發放年度、`$3` 新的所屬年度、`$4` 期別、`$5` 除息日、`$6` 除權日。
/// 新所屬年度若已有自己的一列就不動（那是另一次配息）。
const RELABEL_SAME_EVENT_SQL: &str = r#"
    UPDATE dividend SET year_of_dividend = $3
    WHERE security_code = $1 AND year = $2 AND quarter = $4 AND year_of_dividend <> $3
      AND (("ex-dividend_date1" = $5 AND $5 ~ '^\d{4}-\d{2}-\d{2}$')
        OR ("ex-dividend_date2" = $6 AND $6 ~ '^\d{4}-\d{2}-\d{2}$'))
      AND NOT EXISTS (
          SELECT 1 FROM dividend
          WHERE security_code = $1 AND year = $2 AND year_of_dividend = $3 AND quarter = $4
      )
"#;

/// 儲存或更新股利；混合配息的全年明細改用 A，保留序號供持股紀錄關聯。
///
/// 年度層級列（`quarter = ''`）以發放年度為鍵，分期明細以
/// `(security_code, year, year_of_dividend, quarter)` 為鍵。分期明細若在同一發放年度、
/// 同期別已有一列除權息日相同、只是所屬年度不同的資料，視為同一次配息的所屬年度更正，
/// 先把那一列改成新的所屬年度再 upsert，避免同一次配息變成兩列。
pub(super) async fn save(dividend: &Dividend) -> Result<()> {
    let mut dividend = dividend.clone();
    dividend.clear_dates_of_zero_component();
    let dividend = &dividend;
    let mut tx = database::get_connection().begin().await?;
    if dividend.quarter == "A" {
        // 空季度原本同時代表年配事件與合計；先搬移事件，避免合計覆蓋它。
        // 搬移與 upsert 使用同一交易，失敗時不留下半套資料。
        sqlx::query(
            r#"
            UPDATE dividend SET quarter = 'A'
            WHERE security_code = $1 AND year = $2 AND year_of_dividend = $3
              AND quarter = ''
              AND NOT EXISTS (
                  SELECT 1 FROM dividend
                  WHERE security_code = $1 AND year = $2 AND year_of_dividend = $3 AND quarter = 'A'
              )
        "#,
        )
        .bind(&dividend.security_code)
        .bind(dividend.year)
        .bind(dividend.year_of_dividend)
        .execute(&mut *tx)
        .await?;
    }
    if !dividend.quarter.is_empty() {
        sqlx::query(RELABEL_SAME_EVENT_SQL)
            .bind(&dividend.security_code)
            .bind(dividend.year)
            .bind(dividend.year_of_dividend)
            .bind(&dividend.quarter)
            .bind(&dividend.ex_dividend_date_cash)
            .bind(&dividend.ex_dividend_date_stock)
            .execute(&mut *tx)
            .await
            .context("Failed to relabel the year of dividend")?;
    }
    let sql = if dividend.quarter.is_empty() {
        SAVE_ANNUAL_LEVEL_SQL
    } else {
        SAVE_DETAIL_SQL
    };
    sqlx::query(sql)
        .bind(&dividend.security_code)
        .bind(dividend.year)
        .bind(dividend.year_of_dividend)
        .bind(&dividend.quarter)
        .bind(dividend.cash_dividend)
        .bind(dividend.stock_dividend)
        .bind(dividend.sum)
        .bind(&dividend.ex_dividend_date_cash)
        .bind(&dividend.ex_dividend_date_stock)
        .bind(&dividend.payable_date_cash)
        .bind(&dividend.payable_date_stock)
        .bind(dividend.created_time)
        .bind(Local::now()) // updated_time
        .bind(dividend.capital_reserve_cash_dividend)
        .bind(dividend.earnings_cash_dividend)
        .bind(dividend.capital_reserve_stock_dividend)
        .bind(dividend.earnings_stock_dividend)
        .bind(dividend.payout_ratio_cash)
        .bind(dividend.payout_ratio_stock)
        .bind(dividend.payout_ratio)
        .execute(&mut *tx)
        .await
        .context("Failed to save dividend to database")?;
    tx.commit().await?;
    Ok(())
}
