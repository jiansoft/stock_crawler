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
}

/// 取得指定日期為除息權日的股票
pub async fn fetch_stocks_with_dividends_on_date(
    date: NaiveDate,
) -> Result<Vec<StockDividendInfo>> {
    let sql = r#"
SELECT
    s.stock_symbol,
    s."Name" AS name,
    d.cash_dividend,
    d.stock_dividend,
    d.sum
FROM
    dividend AS d
INNER JOIN
      stocks AS s ON s.stock_symbol = d.security_code
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

    use chrono::{Local, TimeZone};

    use crate::logging;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_fetch_stocks_with_dividends_on_date() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_stocks_with_dividends_on_date".to_string());

        let ex_date = Local.with_ymd_and_hms(2023, 4, 20, 0, 0, 0).unwrap();
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
    }
}
