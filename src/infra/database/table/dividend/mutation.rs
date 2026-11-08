//! `Dividend` 的資料庫寫入／更新操作。
//!
//! 包含單筆 upsert、年度股利合併寫入，以及配息/發放日更新。

use anyhow::{Result, anyhow};
use sqlx::postgres::PgQueryResult;

use crate::infra::database;

use super::Dividend;

/// 年度層級列（`quarter = ''`）的衝突目標：每個發放年度只有一列（部分唯一索引）。
macro_rules! annual_level_conflict_target {
    () => {
        r#"(security_code, "year") WHERE quarter = ''"#
    };
}

/// 分期明細的衝突目標：主鍵 `(security_code, year, year_of_dividend, quarter)`。
///
/// 所屬年度在主鍵內，同一發放年度才能同時存兩次同期別的配息（3008 的 2021H1 與 2022H1）。
macro_rules! detail_conflict_target {
    () => {
        r#"(security_code, "year", year_of_dividend, quarter)"#
    };
}

pub(crate) use {annual_level_conflict_target, detail_conflict_target};

/// 組出 [`Dividend::upsert`] 的 SQL；`$target` 為衝突目標。
macro_rules! upsert_sql {
    ($target:expr) => {
        concat!(
            r#"
INSERT INTO dividend (
    security_code, "year", year_of_dividend, quarter,
    cash_dividend, stock_dividend, "sum","ex-dividend_date1", "ex-dividend_date2",
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
    payout_ratio_cash = EXCLUDED.payout_ratio_cash,
    payout_ratio_stock = EXCLUDED.payout_ratio_stock,
    payout_ratio = EXCLUDED.payout_ratio;
"#
        )
    };
}

/// 年度層級列的 upsert：以發放年度為衝突目標。
const UPSERT_ANNUAL_LEVEL_SQL: &str = upsert_sql!(annual_level_conflict_target!());
/// 分期明細的 upsert：以主鍵為衝突目標。
const UPSERT_DETAIL_SQL: &str = upsert_sql!(detail_conflict_target!());

impl Dividend {
    /// Asynchronously upserts a dividend record into the database.
    ///
    /// This method inserts a new record into the `dividend` table, or updates an existing record if a conflict arises.
    /// Conflicts follow the two unique keys: one annual-level row per payout year,
    /// and `security_code`, `year`, `year_of_dividend`, `quarter` for period rows.
    ///
    /// The method binds the properties of the `Entity` struct to the SQL query parameters and executes the query using the `DB.pool`.
    ///
    /// # Returns
    ///
    /// This method returns a `Result` wrapping a `PgQueryResult`, which represents the result of the query execution.
    /// On success, the `PgQueryResult` includes information about the executed query, such as the number of rows affected.
    /// On failure, the `Result` will contain an `Error`.
    ///
    /// # Errors
    ///
    /// This method will return an error if the SQL query execution fails,
    /// for instance due to a database connection error or a violation of database constraints.
    pub async fn upsert(&self) -> Result<PgQueryResult> {
        let sql = if self.quarter.is_empty() {
            UPSERT_ANNUAL_LEVEL_SQL
        } else {
            UPSERT_DETAIL_SQL
        };
        sqlx::query(sql)
            .bind(&self.security_code)
            .bind(self.year)
            .bind(self.year_of_dividend)
            .bind(&self.quarter)
            .bind(self.cash_dividend)
            .bind(self.stock_dividend)
            .bind(self.sum)
            .bind(&self.ex_dividend_date1)
            .bind(&self.ex_dividend_date2)
            .bind(&self.payable_date1)
            .bind(&self.payable_date2)
            .bind(self.created_time)
            .bind(self.updated_time)
            .bind(self.capital_reserve_cash_dividend)
            .bind(self.earnings_cash_dividend)
            .bind(self.capital_reserve_stock_dividend)
            .bind(self.earnings_stock_dividend)
            .bind(self.payout_ratio_cash)
            .bind(self.payout_ratio_stock)
            .bind(self.payout_ratio)
            .execute(database::get_connection())
            .await
            .map_err(|why| {
                anyhow!(
                    "Failed to upsert({:#?}) from database\nsql:{}\n{:?}",
                    self,
                    sql,
                    why,
                )
            })
    }

    /// 更新年度內有多次配息記錄時將其合併計算成年度股利
    ///
    /// 合計列的日期一律是 `'-'`，衝突更新時必須連日期一起覆寫；原因與
    /// [`DividendRepository::upsert_annual_total_dividend`](crate::domain::dividend::repository::DividendRepository::upsert_annual_total_dividend) 相同。
    pub async fn upsert_annual_total_dividend(&self) -> Result<PgQueryResult> {
        // 使用參數化查詢代替字串格式化，將 $1, $2, $3, $4 分別綁定相關欄位，以移除 AssertSqlSafe
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
            .bind(self.year)
            .bind(self.year - 1)
            .bind(&self.security_code)
            .bind(self.year)
            .execute(database::get_connection())
            .await
            .map_err(|why| {
                anyhow!(
                    "Failed to update_annual_total_dividend({:#?}) from database\nsql:{}\n{:?}",
                    self,
                    sql,
                    why,
                )
            })
    }
}

#[cfg(test)]
mod tests;
