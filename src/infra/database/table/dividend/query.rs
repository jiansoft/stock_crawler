//! `Dividend` 的資料庫查詢操作。
//!
//! 包含依日期彙總股利、年度/股票代號查詢、未公布日期查詢、多次配息查詢等，
//! 以及共用的資料列轉換 helper 與欄位常數。

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use rust_decimal::Decimal;
use sqlx::{Row, postgres::PgRow};

use crate::infra::database;

use super::Dividend;

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

        sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(security_code)
            .bind(year)
            .bind(Local::now().format("%Y-%m-%d %H:%M:%S").to_string())
            .bind(stock_purchase_date.format("%Y-%m-%d %H:%M:%S").to_string())
            .try_map(Self::row_to_entity)
            .fetch_all(database::get_connection())
            .await
            .context(format!(
                "Failed to fetch_dividends_summary_by_date({},{}) from database",
                year, stock_purchase_date
            ))
    }

    /// 取得指定股票目前已有股利資料的發放年度清單。
    ///
    /// 回補持股已領股利時，呼叫端只提供股票代號即可；此查詢會從 `dividend`
    /// 表找出該股票所有已入庫的發放年度，讓後續流程逐年重算持股總表與明細表。
    ///
    /// # 參數
    ///
    /// - `security_code`：要重算股利領取紀錄的股票代號。
    ///
    /// # 錯誤
    ///
    /// 資料庫查詢失敗時會回傳 `Err`，並附上股票代號方便定位。
    pub async fn fetch_years_by_security_code(security_code: &str) -> Result<Vec<i32>> {
        let sql = r#"
SELECT DISTINCT year
FROM dividend
WHERE security_code = $1 AND year > 0
ORDER BY year;
"#;

        sqlx::query_scalar::<_, i32>(sql)
            .bind(security_code)
            .fetch_all(database::get_connection())
            .await
            .context(format!(
                "Failed to fetch_years_by_security_code({}) from database",
                security_code
            ))
    }

    /// 取得指定年度尚未有配息日或發放日的股息數據(有排除配息金額為 0)
    pub async fn fetch_unpublished_dividend_date_or_payable_date_for_specified_year(
        year: i32,
    ) -> Result<Vec<Dividend>> {
        let sql = format!(
            r#"
SELECT {}
FROM
    dividend
WHERE
    (year = $1 OR year_of_dividend = $1)
    AND
    (
        (
            cash_dividend > 0
            AND (
                "ex-dividend_date1" IN ('-', '尚未公布')
                OR payable_date1 IN ('-', '尚未公布')
            )
        )
        OR
        (
            stock_dividend > 0
            AND (
                "ex-dividend_date2" IN ('-', '尚未公布')
                OR payable_date2 IN ('-', '尚未公布')
            )
        )
    );
"#,
            TABLE_COLUMNS
        );
        sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(year)
            .try_map(Self::row_to_entity)
            .fetch_all(database::get_connection())
            .await
            .context(format!(
                "Failed to fetch_unpublished_dividend_date_or_payable_date_for_specified_year({}) from database",
                year
            ))
    }

    /// 取得指定年度相關的多次配息資料。
    ///
    /// 跨年度配息時，`year` 代表實際發放年度，`year_of_dividend` 代表股利所屬年度。
    /// 後續去重 key 使用 `security_code-year_of_dividend-quarter`，所以這裡兩種年度都要納入，
    /// 避免剛跨年度時漏掉已存在的季配或半年配資料。
    pub async fn fetch_multiple_dividends_for_year(year: i32) -> Result<Vec<Dividend>> {
        let sql = format!(
            r#"
SELECT {}
FROM dividend
WHERE (year = $1 OR year_of_dividend = $1) AND quarter IN ('Q1','Q2','Q3','Q4','H1','H2');
"#,
            TABLE_COLUMNS
        );

        sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(year)
            .try_map(Self::row_to_entity)
            .fetch_all(database::get_connection())
            .await
            .context(format!(
                "Failed to fetch_multiple_dividends_for_year({}) from database",
                year
            ))
    }

    /// 取得尚未有指定年度配息的股票代號
    pub async fn fetch_no_dividends_for_year(year: i32) -> Result<Vec<String>> {
        let sql = r#"
SELECT
    stock_symbol
FROM stocks
WHERE "SuspendListing" = false
    AND stock_exchange_market_id IN (2, 4)
    AND stock_symbol NOT IN 
        (SELECT security_code FROM dividend WHERE year = $1 AND quarter = '');
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

    /*    /// 取得尚未有指定年度配息的股票代號
    pub async fn fetch_stock_symbol_that_without_payout_ratio() -> Result<Vec<String>> {
        let sql = r#"
    SELECT
        security_code
    FROM dividend
    WHERE payout_ratio = 0
    GROUP BY security_code
    ORDER BY random();
    "#;
        let stock_symbols: Vec<String> = sqlx::query(sql)
            .fetch_all(database::get_connection())
            .await?
            .into_iter()
            .map(|row| row.get("security_code"))
            .collect();

        Ok(stock_symbols)
    }*/

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

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[tokio::test]
    async fn test_fetch_no_dividends_for_year() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 fetch_no_dividends_for_year");
        let r = Dividend::fetch_no_dividends_for_year(2023).await;
        if let Ok(result) = r {
            for e in result {
                tracing::debug!("{:?} ", e);
            }
        } else if let Err(err) = r {
            tracing::debug!("{:#?} ", err);
        }
        tracing::debug!("結束 fetch_no_dividends_for_year");
    }

    #[tokio::test]

    async fn test_fetch_unpublished_dividend_date_or_payable_date_for_specified_year() {
        dotenvy::dotenv().ok();
        tracing::debug!(
            "{}",
            "開始 fetch_unpublished_dividend_date_or_payable_date_for_specified_year".to_string(),
        );
        let r = Dividend::fetch_unpublished_dividend_date_or_payable_date_for_specified_year(2023)
            .await;
        if let Ok(result) = r {
            tracing::debug!("{:#?} ", result);
        } else if let Err(err) = r {
            tracing::debug!("{:#?} ", err);
        }
        tracing::debug!(
            "{}",
            "結束 fetch_unpublished_dividend_date_or_payable_date_for_specified_year".to_string(),
        );
    }

    #[tokio::test]
    async fn test_fetch_multiple_dividends_for_year() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 fetch_multiple_dividends_for_year");
        let r = Dividend::fetch_multiple_dividends_for_year(2023).await;
        if let Ok(result) = r {
            for e in result {
                tracing::debug!("{:?} ", e);
            }
        } else if let Err(err) = r {
            tracing::debug!("{:#?} ", err);
        }
        tracing::debug!("結束 fetch_multiple_dividends_for_year");
    }

    #[tokio::test]
    async fn test_fetch_dividends_summary_by_date() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 fetch_dividends_summary_by_date");

        let datetime = Local.with_ymd_and_hms(2022, 3, 9, 0, 0, 0).unwrap();

        let r = Dividend::fetch_dividends_summary_by_date("2330", 2022, datetime).await;
        if let Ok(result) = r {
            for e in result {
                tracing::debug!("{:?} ", e);
            }
        } else if let Err(err) = r {
            tracing::debug!("{:#?} ", err);
        }
        tracing::debug!("結束 fetch_dividends_summary_by_date");
    }

    #[tokio::test]
    async fn test_fetch_yearly_dividends_sum_by_date() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 fetch_yearly_dividends_sum_by_date");
        let mut e = Dividend::new();
        e.security_code = "2887".to_string();
        e.year = 2022;
        let datetime = Local.with_ymd_and_hms(2022, 3, 9, 0, 0, 0).unwrap();

        let r = e.fetch_yearly_dividends_sum_by_date(datetime).await;
        if let Ok(result) = r {
            tracing::debug!("{:?} {:?}", e, result);
        } else if let Err(err) = r {
            tracing::debug!("{:#?} ", err);
        }
        tracing::debug!("結束 fetch_yearly_dividends_sum_by_date");
    }
}
