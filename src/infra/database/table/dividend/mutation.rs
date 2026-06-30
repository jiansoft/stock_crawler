//! `Dividend` 的資料庫寫入／更新操作。
//!
//! 包含單筆 upsert、批次 upsert、年度股利合併寫入，以及配息/發放日更新。

use anyhow::{Context, Result, anyhow};
use sqlx::{QueryBuilder, postgres::PgQueryResult};

use crate::infra::database;

use super::Dividend;

impl Dividend {
    /// Asynchronously upserts a dividend record into the database.
    ///
    /// This method inserts a new record into the `dividend` table, or updates an existing record if a conflict arises.
    /// Conflicts are determined by a combination of `security_code`, `year`, and `quarter`.
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
        let sql = r#"
INSERT INTO dividend (
    security_code, "year", year_of_dividend, quarter,
    cash_dividend, stock_dividend, "sum","ex-dividend_date1", "ex-dividend_date2",
    payable_date1, payable_date2, created_time, updated_time, capital_reserve_cash_dividend,
    earnings_cash_dividend, capital_reserve_stock_dividend, earnings_stock_dividend,
    payout_ratio_cash, payout_ratio_stock, payout_ratio)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20)
ON CONFLICT (security_code,"year",quarter) DO UPDATE SET
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
"#;
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

    /// 批次新增或更新多筆股利資料（以 `security_code + year + quarter` 為鍵）。
    ///
    /// 此函式使用 SQLx 的 `QueryBuilder::push_values` 建立多值 `INSERT` 語句，
    /// 並在主鍵衝突時執行對應的欄位更新。相較於單筆逐次寫入，批次寫入能顯著降低資料庫連線延遲。
    ///
    /// # 參數
    /// * `dividends` - 要寫入的股利實體清單。
    ///
    /// # 錯誤
    /// 當 SQL 執行失敗或傳入清單為空時回傳錯誤。
    pub async fn batch_upsert(dividends: &[Self]) -> Result<PgQueryResult> {
        // 檢查傳入的股利清單是否為空
        if dividends.is_empty() {
            return Err(anyhow!("Cannot batch_upsert empty dividends slice"));
        }

        // 1. 初始化 QueryBuilder 並定義基本 INSERT 欄位
        let mut query_builder = QueryBuilder::new(
            r#"
INSERT INTO dividend (
    security_code, "year", year_of_dividend, quarter,
    cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2",
    payable_date1, payable_date2, created_time, updated_time, capital_reserve_cash_dividend,
    earnings_cash_dividend, capital_reserve_stock_dividend, earnings_stock_dividend,
    payout_ratio_cash, payout_ratio_stock, payout_ratio)
"#,
        );

        // 2. 批次推入資料列與對應參數綁定
        query_builder.push_values(dividends, |mut b, item| {
            b.push_bind(&item.security_code)
                .push_bind(item.year)
                .push_bind(item.year_of_dividend)
                .push_bind(&item.quarter)
                .push_bind(item.cash_dividend)
                .push_bind(item.stock_dividend)
                .push_bind(item.sum)
                .push_bind(&item.ex_dividend_date1)
                .push_bind(&item.ex_dividend_date2)
                .push_bind(&item.payable_date1)
                .push_bind(&item.payable_date2)
                .push_bind(item.created_time)
                .push_bind(item.updated_time)
                .push_bind(item.capital_reserve_cash_dividend)
                .push_bind(item.earnings_cash_dividend)
                .push_bind(item.capital_reserve_stock_dividend)
                .push_bind(item.earnings_stock_dividend)
                .push_bind(item.payout_ratio_cash)
                .push_bind(item.payout_ratio_stock)
                .push_bind(item.payout_ratio);
        });

        // 3. 串接衝突更新子句 (Conflict Resolution)
        query_builder.push(
            r#"
ON CONFLICT (security_code,"year",quarter) DO UPDATE SET
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
"#,
        );

        // 4. 建立並執行查詢
        let query = query_builder.build();
        query
            .execute(database::get_connection())
            .await
            .map_err(|why| anyhow!("Failed to batch_upsert dividends from database: {:?}", why))
    }

    /// 更新年度內有多次配息記錄時將其合併計算成年度股利
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
ON CONFLICT (security_code,year,quarter) DO UPDATE SET
    cash_dividend = EXCLUDED.cash_dividend,
    stock_dividend = EXCLUDED.stock_dividend,
    sum = EXCLUDED.sum;;
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

        tracing::debug!("結束 upsert");
    }

    /// 驗證 `upsert` 在主鍵衝突時會同步更新除權息日與股利發放日。
    ///
    /// 此測試會實際寫入 `dividend` 表。先用測試股票代碼寫入 `尚未公布` 日期，再以同一主鍵
    /// upsert 正式日期，最後查回確認四個日期欄位都已被覆蓋。測試結束會刪除測試股票代碼資料。
    #[tokio::test]
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
