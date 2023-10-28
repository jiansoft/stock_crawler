use anyhow::{Context, Result};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use sqlx::FromRow;

use crate::internal::database;

/// 股票除息的資料
#[derive(FromRow, Debug)]
pub struct StockDividendPayableDateInfo {
    pub stock_symbol: String,
    pub name: String,
    pub cash_dividend: Decimal,
    pub stock_dividend: Decimal,
    pub sum: Decimal,
    pub payable_date1: String,
    pub payable_date2: String,
}

/// 取得指定日期為股利發放日的股票
pub async fn fetch(date: NaiveDate) -> Result<Vec<StockDividendPayableDateInfo>> {
    let sql = r#"
SELECT
    s.stock_symbol,
    s."Name" AS name,
    d.cash_dividend,
    d.stock_dividend,
    d.sum,
    d."payable_date1",
    d."payable_date2"
FROM
    dividend AS d
INNER JOIN
    stocks AS s ON s.stock_symbol = d.security_code
WHERE security_code in (select security_code from stock_ownership_details where is_sold = false)
    AND (d."payable_date1" = $1 OR d."payable_date2" = $1);
"#;

    sqlx::query_as::<_, StockDividendPayableDateInfo>(sql)
        .bind(date.format("%Y-%m-%d").to_string())
        .fetch_all(database::get_connection())
        .await
        .context(format!(
            "Failed to StockDividendPayableDateInfo::fetch({}) from database",
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
    async fn test_fetch_stocks_with_payable_on_date() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_stocks_with_payable_on_date".to_string());

        let ex_date = Local.with_ymd_and_hms(2023, 8, 25, 0, 0, 0).unwrap();
        let d = ex_date.date_naive();
        match fetch(d).await {
            Ok(cd) => {
                dbg!(&cd);
                logging::debug_file_async(format!("stock: {:?}", cd));
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to fetch_stocks_with_payable_on_date because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 fetch_stocks_with_payable_on_date".to_string());
    }
}
