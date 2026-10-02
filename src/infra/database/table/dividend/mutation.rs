//! `Dividend` 的資料庫寫入／更新操作。
//!
//! 包含單筆 upsert、年度股利合併寫入，以及配息/發放日更新。

use anyhow::{Context, Result, anyhow};
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
    /// [`crate::infra::database::repository::dividend::PgDividendRepository::upsert_annual_total_dividend`] 相同。
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

    /// 更新股息的配息日、發放日
    pub async fn update_dividend_date(&self) -> Result<PgQueryResult> {
        let sql = r#"
UPDATE
    dividend
SET
    "ex-dividend_date1" = $2,
    "ex-dividend_date2" = $3,
    payable_date1 = $4,
    payable_date2 = $5,
    updated_time = NOW()
WHERE
    serial = $1;
"#;
        sqlx::query(sql)
            .bind(self.serial)
            .bind(&self.ex_dividend_date1)
            .bind(&self.ex_dividend_date2)
            .bind(&self.payable_date1)
            .bind(&self.payable_date2)
            .execute(database::get_connection())
            .await
            .context(format!(
                "Failed to update_dividend_date({:#?}) from database",
                self
            ))
    }
}

#[cfg(test)]
mod tests {
    use chrono::Local;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;
    use sqlx::Row;

    use super::*;

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_upsert() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 upsert");
        let mut e = Dividend::new();
        e.security_code = String::from("79979");
        e.year = 2023;
        e.year_of_dividend = 2023;
        e.quarter = String::from("H1");
        e.ex_dividend_date1 = "尚未公布".to_string();
        e.ex_dividend_date2 = "尚未公布".to_string();
        e.payable_date1 = "尚未公布".to_string();
        e.payable_date2 = "尚未公布".to_string();
        e.created_time = Local::now();
        e.updated_time = Local::now();
        e.cash_dividend = dec!(1);
        e.stock_dividend = dec!(2);
        e.sum = dec!(3);
        e.capital_reserve_cash_dividend = dec!(0.5);
        e.earnings_cash_dividend = dec!(0.5);
        e.capital_reserve_stock_dividend = dec!(1);
        e.earnings_stock_dividend = dec!(1);
        e.payout_ratio = dec!(99);
        e.payout_ratio_cash = dec!(45);
        e.payout_ratio_stock = dec!(44);

        match e.upsert().await {
            Ok(result) => {
                tracing::debug!("{:?} {:?} ", result, e);
            }
            Err(why) => {
                tracing::debug!("Failed to upsert because {:?} ", why);
            }
        }

        // 假代號的測試資料必須清除，否則會殘留在資料庫裡影響其他查詢。
        let _ = sqlx::query("DELETE FROM dividend WHERE security_code = $1 AND year = $2")
            .bind(&e.security_code)
            .bind(e.year)
            .execute(database::get_connection())
            .await;

        tracing::debug!("結束 upsert");
    }

    /// 驗證 `upsert` 在主鍵衝突時會同步更新除權息日與股利發放日。
    ///
    /// 此測試會實際寫入 `dividend` 表。先用測試股票代碼寫入 `尚未公布` 日期，再以同一主鍵
    /// upsert 正式日期，最後查回確認四個日期欄位都已被覆蓋。測試結束會刪除測試股票代碼資料。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_upsert_updates_dividend_dates_on_conflict() {
        dotenvy::dotenv().ok();

        let security_code = "__TEST_UPSERT_DATE__";
        let year = 2099;
        let cleanup_sql = "DELETE FROM dividend WHERE security_code = $1;";

        sqlx::query(cleanup_sql)
            .bind(security_code)
            .execute(database::get_connection())
            .await
            .expect("cleanup dividend test rows before test");

        let mut unpublished = Dividend::new();
        unpublished.security_code = security_code.to_string();
        unpublished.year = year;
        unpublished.year_of_dividend = year - 1;
        unpublished.cash_dividend = dec!(42);
        unpublished.sum = dec!(42);
        unpublished.ex_dividend_date1 = "尚未公布".to_string();
        unpublished.ex_dividend_date2 = "-".to_string();
        unpublished.payable_date1 = "尚未公布".to_string();
        unpublished.payable_date2 = "-".to_string();
        unpublished
            .upsert()
            .await
            .expect("insert unpublished dividend row");

        let mut published = unpublished.clone();
        published.cash_dividend = dec!(43);
        published.sum = dec!(43);
        published.ex_dividend_date1 = "2099-06-26".to_string();
        published.ex_dividend_date2 = "2099-06-27".to_string();
        published.payable_date1 = "2099-07-17".to_string();
        published.payable_date2 = "2099-07-18".to_string();
        published
            .upsert()
            .await
            .expect("update published dividend row");

        let row = sqlx::query(
            r#"
SELECT cash_dividend,
       "ex-dividend_date1",
       "ex-dividend_date2",
       payable_date1,
       payable_date2
FROM dividend
WHERE security_code = $1 AND year = $2 AND quarter = '';
"#,
        )
        .bind(security_code)
        .bind(year)
        .fetch_one(database::get_connection())
        .await
        .expect("fetch updated dividend row");

        assert_eq!(row.get::<Decimal, _>("cash_dividend"), dec!(43));
        assert_eq!(row.get::<String, _>("ex-dividend_date1"), "2099-06-26");
        assert_eq!(row.get::<String, _>("ex-dividend_date2"), "2099-06-27");
        assert_eq!(row.get::<String, _>("payable_date1"), "2099-07-17");
        assert_eq!(row.get::<String, _>("payable_date2"), "2099-07-18");

        sqlx::query(cleanup_sql)
            .bind(security_code)
            .execute(database::get_connection())
            .await
            .expect("cleanup dividend test rows after test");
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_upsert_annual_total_dividend_operates_database() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 upsert_annual_total_dividend");

        // 此函式 SQL 只使用股票代號與發放年度兩個參數；測試只補齊必要參數並確認 SQL 可執行。
        let mut annual_total_seed = Dividend::new();
        annual_total_seed.security_code = "5306".to_string();
        annual_total_seed.year = 2026;

        annual_total_seed
            .upsert_annual_total_dividend()
            .await
            .expect("upsert annual total dividend");

        tracing::debug!("結束 upsert_annual_total_dividend");
    }
}
