use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use sqlx::FromRow;

use crate::database;

/// 股票除息的資料
#[derive(FromRow, Debug)]
pub struct StockDividendInfo {
    pub stock_symbol: String,
    pub name: String,
    pub cash_dividend: Decimal,
    pub stock_dividend: Decimal,
    pub sum: Decimal,
    pub closing_price: Decimal,
    pub dividend_yield: Decimal,
    pub cash_dividend_yield: Decimal,
}

/// 取得指定日期為除息權日的股票
pub async fn fetch_stocks_with_dividends_on_date(
    date: NaiveDate,
) -> Result<Vec<StockDividendInfo>> {
    let sql = r#"
SELECT
       s.stock_symbol,
       s."Name"                                 AS name,
       ROUND(d.cash_dividend, 2) as cash_dividend,
       ROUND(d.stock_dividend, 2) as stock_dividend,
       ROUND(d.sum, 2) as sum,
       COALESCE(ROUND(ldq.closing_price, 2), 0) as closing_price,
       CASE
           WHEN ldq.closing_price IS NULL THEN 0
           ELSE ROUND((d.sum / ldq.closing_price) * 100, 2)
           END                                  AS dividend_yield,
       CASE
           WHEN ldq.closing_price IS NULL THEN 0
           ELSE ROUND((d.cash_dividend / ldq.closing_price) * 100, 2)
           END                                  AS cash_dividend_yield
FROM
    dividend AS d
INNER JOIN
      stocks AS s ON s.stock_symbol = d.security_code
LEFT JOIN 
last_daily_quotes AS ldq on d.security_code = ldq.security_code      
WHERE
    d."year" = $1
    AND (d."ex-dividend_date1" = $2 OR d."ex-dividend_date2" = $2);
"#;

    sqlx::query_as::<_, StockDividendInfo>(sql)
        .bind(date.year())
        .bind(date.format("%Y-%m-%d").to_string())
        .fetch_all(database::get_connection())
        .await
        .context(format!(
            "Failed to fetch_stocks_with_dividends_on_date({}) from database",
            date
        ))
}

#[cfg(test)]
mod tests {
    use core::result::Result::Ok;
    use std::time::Duration;

    use chrono::{Local, TimeZone};
    use tokio::time;

    use crate::logging;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_fetch_stocks_with_dividends_on_date() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_stocks_with_dividends_on_date".to_string());

        let ex_date = Local.with_ymd_and_hms(2024, 6, 21, 0, 0, 0).unwrap();
        let d = ex_date.date_naive();
        match fetch_stocks_with_dividends_on_date(d).await {
            Ok(cd) => {
                dbg!(&cd);
                logging::debug_file_async(format!("stock: {:?}", cd));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to execute because {:?}", why));
            }
        }

        logging::debug_file_async("結束 fetch_stocks_with_dividends_on_date".to_string());
        time::sleep(Duration::from_secs(1)).await;
    }
}
