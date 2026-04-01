use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use sqlx::FromRow;

use crate::database;

/// 指定日期有除權或除息事件的股票資料。
///
/// 此結構同時提供：
/// 1. 第一則 Telegram 清單所需的展示欄位。
/// 2. 第二則「持股預估股利」訊息所需的原始股利數值與事件旗標。
#[derive(FromRow, Debug, Clone)]
pub struct StockDividendInfo {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 現金股利（元）。
    pub cash_dividend: Decimal,
    /// 股票股利（股）。
    pub stock_dividend: Decimal,
    /// 股利合計（元）。
    pub sum: Decimal,
    /// 參考收盤價。
    pub closing_price: Decimal,
    /// 總殖利率（%）。
    pub dividend_yield: Decimal,
    /// 現金殖利率（%）。
    pub cash_dividend_yield: Decimal,
    /// 是否於指定日期進行除息。
    pub is_cash_ex_dividend_today: bool,
    /// 是否於指定日期進行除權。
    pub is_stock_ex_dividend_today: bool,
}

/// 取得指定日期為除權或除息日的股票。
///
/// 這裡保留 `cash_dividend` / `stock_dividend` / `sum` 的原始精度，
/// 避免先在 SQL 四捨五入後又乘上持股股數，造成預估股利累積誤差。
pub async fn fetch_stocks_with_dividends_on_date(
    date: NaiveDate,
) -> Result<Vec<StockDividendInfo>> {
    let sql = r#"
SELECT
       s.stock_symbol,
       s."Name"                                 AS name,
       d.cash_dividend,
       d.stock_dividend,
       d.sum,
       COALESCE(ROUND(ldq.closing_price, 2), 0) as closing_price,
       CASE
           WHEN ldq.closing_price IS NULL THEN 0
           ELSE ROUND((d.sum / ldq.closing_price) * 100, 2)
           END                                  AS dividend_yield,
       CASE
           WHEN ldq.closing_price IS NULL THEN 0
           ELSE ROUND((d.cash_dividend / ldq.closing_price) * 100, 2)
           END                                  AS cash_dividend_yield,
       d."ex-dividend_date1" = $2               AS is_cash_ex_dividend_today,
       d."ex-dividend_date2" = $2               AS is_stock_ex_dividend_today
FROM
    dividend AS d
INNER JOIN
      stocks AS s ON s.stock_symbol = d.security_code
LEFT JOIN 
last_daily_quotes AS ldq on s.stock_symbol = ldq.stock_symbol      
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
