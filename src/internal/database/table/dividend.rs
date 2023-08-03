use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use rust_decimal::Decimal;
use sqlx::{
    postgres::{PgQueryResult, PgRow},
    Row,
};

use crate::internal::{crawler::goodinfo, database};

#[derive(sqlx::Type, sqlx::FromRow, Debug, Clone)]
/// 股息發放日程表 原表名 dividend
pub struct Dividend {
    /// 序號
    pub serial: i64,
    /// 發放年度
    pub year: i32,
    /// 股利所屬年度
    pub year_of_dividend: i32,
    /// 發放季度
    pub quarter: String,
    /// 股票代號
    pub security_code: String,
    /// 盈餘現金股利 (Cash Dividend)
    pub earnings_cash_dividend: Decimal,
    /// 公積現金股利 (Capital Reserve)
    pub capital_reserve_cash_dividend: Decimal,
    /// 現金股利合計
    pub cash_dividend: Decimal,
    /// 盈餘股票股利 (Stock Dividend)
    pub earnings_stock_dividend: Decimal,
    /// 公積股票股利 (Capital Reserve)
    pub capital_reserve_stock_dividend: Decimal,
    /// 股票股利合計
    pub stock_dividend: Decimal,
    /// 合計股利(元)
    pub sum: Decimal,
    /// 盈餘分配率_配息(%)
    pub payout_ratio_cash: Decimal,
    /// 盈餘分配率_配股(%)
    pub payout_ratio_stock: Decimal,
    /// 盈餘分配率(%)
    pub payout_ratio: Decimal,
    /// 除息日
    pub ex_dividend_date1: String,
    /// 除權日
    pub ex_dividend_date2: String,
    /// 現金股利發放日
    pub payable_date1: String,
    /// 股票股利發放日
    pub payable_date2: String,
    pub created_time: DateTime<Local>,
    pub updated_time: DateTime<Local>,
}

const TABLE_COLUMNS: &str = r#"
    serial,
    security_code,
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
    payout_ratio"#;

impl Dividend {
    pub fn new() -> Self {
        Dividend {
            serial: 0,
            year: 0,
            year_of_dividend: 0,
            quarter: "".to_string(),
            security_code: Default::default(),
            earnings_cash_dividend: Default::default(),
            capital_reserve_cash_dividend: Default::default(),
            cash_dividend: Default::default(),
            earnings_stock_dividend: Default::default(),
            capital_reserve_stock_dividend: Default::default(),
            stock_dividend: Default::default(),
            sum: Default::default(),
            payout_ratio_cash: Default::default(),
            payout_ratio_stock: Default::default(),
            payout_ratio: Default::default(),
            ex_dividend_date1: "".to_string(),
            ex_dividend_date2: "".to_string(),
            payable_date1: "".to_string(),
            payable_date2: "".to_string(),
            created_time: Local::now(),
            updated_time: Local::now(),
        }
    }

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
    updated_time = EXCLUDED.updated_time,
    capital_reserve_cash_dividend = EXCLUDED.capital_reserve_cash_dividend,
    earnings_cash_dividend = EXCLUDED.earnings_cash_dividend,
    capital_reserve_stock_dividend = EXCLUDED.capital_reserve_stock_dividend,
    earnings_stock_dividend = EXCLUDED.earnings_stock_dividend,
    payout_ratio_cash = EXCLUDED.payout_ratio_cash,
    payout_ratio_stock = EXCLUDED.payout_ratio_stock,
    payout_ratio = EXCLUDED.payout_ratio;
"#;
        let result = sqlx::query(sql)
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
            .await?;

        Ok(result)
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
            .context("Failed to update_dividend_date from database")
    }

    /// 按照年份和除權息日取得股利總和的數據
    pub async fn fetch_yearly_dividends_sum_by_date(
        &self,
        stock_purchase_date: DateTime<Local>,
    ) -> Result<(Decimal, Decimal, Decimal)> {
        let entities = Self::fetch_dividends_summary_by_date(
            &self.security_code,
            self.year,
            stock_purchase_date,
        )
        .await?;
        let (cash, stock, sum) = entities.into_iter().fold(
            (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO),
            |(acc_cash, acc_stock, acc_sum), entity| {
                (
                    acc_cash + entity.cash_dividend,
                    acc_stock + entity.stock_dividend,
                    acc_sum + entity.sum,
                )
            },
        );

        Ok((cash, stock, sum))
    }

    /// 按照年份和除權息日取得數據
    pub async fn fetch_dividends_summary_by_date(
        security_code: &str,
        year: i32,
        stock_purchase_date: DateTime<Local>,
    ) -> Result<Vec<Dividend>> {
        let sql = format!(
            r#"
select {}
from dividend
where security_code = $1
    and year = $2
    and ("ex-dividend_date1" <= $3)
    and ("ex-dividend_date1" >= $4 or "ex-dividend_date2" >= $4);
"#,
            TABLE_COLUMNS
        );
        let entities = sqlx::query(&sql)
            .bind(security_code)
            .bind(year)
            .bind(Local::now().format("%Y-%m-%d %H:%M:%S").to_string())
            .bind(stock_purchase_date.format("%Y-%m-%d %H:%M:%S").to_string())
            .try_map(Self::row_to_entity)
            .fetch_all(database::get_connection())
            .await?;

        Ok(entities)
    }

    /// 取得指定年度尚未有配息日的配息數據(有排除配息金額為 0)
    pub async fn fetch_unpublished_dividends_for_year(year: i32) -> Result<Vec<Dividend>> {
        let sql = format!(
            r#"
SELECT {}
FROM
    dividend
WHERE
    year = $1 and ("ex-dividend_date1" = '尚未公布' or "ex-dividend_date2" = '尚未公布') and "sum" <> 0;
"#,
            TABLE_COLUMNS
        );

        let entities = sqlx::query(&sql)
            .bind(year)
            .try_map(Self::row_to_entity)
            .fetch_all(database::get_connection())
            .await?;

        Ok(entities)
    }

    /// 取得指定年度內有多次配息的配息資料
    pub async fn fetch_multiple_dividends_for_year(year: i32) -> Result<Vec<Dividend>> {
        let sql = format!(
            r#"
SELECT {}
FROM dividend
WHERE year = $1 AND quarter IN ('Q1','Q2','Q3','Q4','H1','H2');
"#,
            TABLE_COLUMNS
        );

        let entities: Vec<Dividend> = sqlx::query(&sql)
            .bind(year)
            .try_map(Self::row_to_entity)
            .fetch_all(database::get_connection())
            .await?;

        Ok(entities)
    }

    /// 取得尚未有指定年度配息的股票代號
    pub async fn fetch_no_dividends_for_year(year: i32) -> Result<Vec<String>> {
        let sql = r#"
SELECT
    stock_symbol
FROM stocks
WHERE "SuspendListing" = false
    AND stock_exchange_market_id IN (2, 4)
    AND stock_symbol NOT IN (
        SELECT security_code
        FROM dividend
        WHERE year = $1 AND quarter = ''
    );
"#;
        let stock_symbols: Vec<String> = sqlx::query(sql)
            .bind(year)
            .fetch_all(database::get_connection())
            .await?
            .into_iter()
            .map(|row| row.get("stock_symbol"))
            .collect();

        Ok(stock_symbols)
    }

    fn row_to_entity(row: PgRow) -> Result<Dividend, sqlx::Error> {
        Ok(Dividend {
            serial: row.try_get("serial")?,
            security_code: row.try_get("security_code")?,
            year: row.try_get("year")?,
            year_of_dividend: row.try_get("year_of_dividend")?,
            quarter: row.try_get("quarter")?,
            cash_dividend: row.try_get("cash_dividend")?,
            stock_dividend: row.try_get("stock_dividend")?,
            sum: row.try_get("sum")?,
            ex_dividend_date1: row.try_get("ex-dividend_date1")?,
            ex_dividend_date2: row.try_get("ex-dividend_date2")?,
            payable_date1: row.try_get("payable_date1")?,
            payable_date2: row.try_get("payable_date2")?,
            created_time: row.try_get("created_time")?,
            updated_time: row.try_get("updated_time")?,
            capital_reserve_cash_dividend: row.try_get("capital_reserve_cash_dividend")?,
            earnings_cash_dividend: row.try_get("earnings_cash_dividend")?,
            capital_reserve_stock_dividend: row.try_get("capital_reserve_stock_dividend")?,
            earnings_stock_dividend: row.try_get("earnings_stock_dividend")?,
            payout_ratio_cash: row.try_get("payout_ratio_cash")?,
            payout_ratio_stock: row.try_get("payout_ratio_stock")?,
            payout_ratio: row.try_get("payout_ratio")?,
        })
    }
}

impl Default for Dividend {
    fn default() -> Self {
        Self::new()
    }
}
/*
impl Clone for Entity {
    fn clone(&self) -> Self {
        Entity {
            serial: self.serial,
            year: self.year,
            year_of_dividend: self.year_of_dividend,
            quarter: self.quarter.to_string(),
            security_code: self.security_code.to_string(),
            earnings_cash_dividend: self.earnings_cash_dividend,
            capital_reserve_cash_dividend: self.capital_reserve_cash_dividend,
            cash_dividend: self.cash_dividend,
            earnings_stock_dividend: self.earnings_stock_dividend,
            capital_reserve_stock_dividend: self.capital_reserve_stock_dividend,
            stock_dividend: self.stock_dividend,
            sum: self.sum,
            payout_ratio_cash: self.payout_ratio_cash,
            payout_ratio_stock: self.payout_ratio_stock,
            payout_ratio: self.payout_ratio,
            ex_dividend_date1: self.ex_dividend_date1.to_string(),
            ex_dividend_date2: self.ex_dividend_date2.to_string(),
            payable_date1: self.payable_date1.to_string(),
            payable_date2: self.payable_date2.to_string(),
            create_time: self.create_time,
            update_time: self.update_time,
        }
    }
}
*/

//let entity: Entity = fs.into(); // 或者 let entity = Entity::from(fs);
impl From<&goodinfo::dividend::GoodInfoDividend> for Dividend {
    fn from(d: &goodinfo::dividend::GoodInfoDividend) -> Self {
        let mut e = Dividend::new();
        e.quarter = d.quarter.clone();
        e.year = d.year;
        e.year_of_dividend = d.year_of_dividend;
        e.security_code = d.stock_symbol.clone();
        e.earnings_cash_dividend = d.earnings_cash;
        e.capital_reserve_cash_dividend = d.capital_reserve_cash;
        e.cash_dividend = d.cash_dividend;
        e.earnings_stock_dividend = d.earnings_stock;
        e.capital_reserve_stock_dividend = d.capital_reserve_stock;
        e.stock_dividend = d.stock_dividend;
        e.sum = d.sum;
        e.payout_ratio_cash = d.payout_ratio_cash;
        e.payout_ratio_stock = d.payout_ratio_stock;
        e.payout_ratio = d.payout_ratio;
        e.ex_dividend_date1 = d.ex_dividend_date1.clone();
        e.ex_dividend_date2 = d.ex_dividend_date2.clone();
        e.payable_date1 = d.payable_date1.clone();
        e.payable_date2 = d.payable_date2.clone();
        e.created_time = Local::now();
        e.updated_time = Local::now();
        e
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use rust_decimal_macros::dec;

    use crate::internal::logging;

    use super::*;

    #[tokio::test]
    async fn test_fetch_no_dividends_for_year() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_no_dividends_for_year".to_string());
        let r = Dividend::fetch_no_dividends_for_year(2023).await;
        if let Ok(result) = r {
            for e in result {
                logging::debug_file_async(format!("{:?} ", e));
            }
        } else if let Err(err) = r {
            logging::debug_file_async(format!("{:#?} ", err));
        }
        logging::debug_file_async("結束 fetch_no_dividends_for_year".to_string());
    }

    #[tokio::test]
    async fn test_fetch_unannounced_date() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_dividend_unannounced_date".to_string());
        let r = Dividend::fetch_unpublished_dividends_for_year(2023).await;
        if let Ok(result) = r {
            for e in result {
                logging::debug_file_async(format!("{:?} ", e));
            }
        } else if let Err(err) = r {
            logging::debug_file_async(format!("{:#?} ", err));
        }
        logging::debug_file_async("結束 fetch_dividend_unannounced_date".to_string());
    }

    #[tokio::test]
    async fn test_fetch_multiple_dividends_for_year() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_multiple_dividends_for_year".to_string());
        let r = Dividend::fetch_multiple_dividends_for_year(2023).await;
        if let Ok(result) = r {
            for e in result {
                logging::debug_file_async(format!("{:?} ", e));
            }
        } else if let Err(err) = r {
            logging::debug_file_async(format!("{:#?} ", err));
        }
        logging::debug_file_async("結束 fetch_multiple_dividends_for_year".to_string());
    }

    #[tokio::test]
    async fn test_fetch_dividends_summary_by_date() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_dividends_summary_by_date".to_string());

        let datetime = Local.with_ymd_and_hms(2022, 3, 9, 0, 0, 0).unwrap();

        let r = Dividend::fetch_dividends_summary_by_date("2330", 2022, datetime).await;
        if let Ok(result) = r {
            for e in result {
                logging::debug_file_async(format!("{:?} ", e));
            }
        } else if let Err(err) = r {
            logging::debug_file_async(format!("{:#?} ", err));
        }
        logging::debug_file_async("結束 fetch_dividends_summary_by_date".to_string());
    }

    #[tokio::test]
    async fn test_fetch_yearly_dividends_sum_by_date() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_yearly_dividends_sum_by_date".to_string());
        let mut e = Dividend::new();
        e.security_code = "2887".to_string();
        e.year = 2022;
        let datetime = Local.with_ymd_and_hms(2022, 3, 9, 0, 0, 0).unwrap();

        let r = e.fetch_yearly_dividends_sum_by_date(datetime).await;
        if let Ok(result) = r {
            logging::debug_file_async(format!("{:?} {:?}", e, result));
        } else if let Err(err) = r {
            logging::debug_file_async(format!("{:#?} ", err));
        }
        logging::debug_file_async("結束 fetch_yearly_dividends_sum_by_date".to_string());
    }

    #[tokio::test]
    async fn test_upsert() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 upsert".to_string());
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
                logging::debug_file_async(format!("{:?} {:?} ", result, e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to upsert because {:?} ", why));
            }
        }

        logging::debug_file_async("結束 upsert".to_string());
    }
}
