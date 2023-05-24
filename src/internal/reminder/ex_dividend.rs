use crate::internal::{bot, calculation, database::DB, logging};
use chrono::{Datelike, Local, NaiveDate};
use sqlx::FromRow;
use std::fmt::Write;

#[derive(FromRow, Debug)]
struct StockEntity {
    stock_symbol: String,
    name: String,
}

/// 提醒本日為除權息的股票有那些
pub async fn execute() {
    let today: NaiveDate = Local::now().date_naive();
    let year = today.year();
    let date_str = today.format("%Y-%m-%d").to_string();
    logging::info_file_async(format!("ex_dividend date:{}", date_str));

    let sql = r#"
select s.stock_symbol,s."Name" as name
from dividend as d
inner join stocks as s on s.stock_symbol = d.security_code
where "year" = $1 and ("ex-dividend_date1" = $2 or "ex-dividend_date2" = $2)
        "#;

    match sqlx::query_as::<_, StockEntity>(sql)
        .bind(year)
        .bind(&date_str)
        .fetch_all(&DB.pool)
        .await
    {
        Ok(stocks) => {
            if stocks.is_empty() {
                return;
            }
            let mut stock_symbols: Vec<String> = Vec::with_capacity(stocks.len());

            let mut msg = String::with_capacity(2048);
            if writeln!(&mut msg, "{} 進行除權息的股票如下︰", date_str).is_ok() {
                for stock in stocks {
                    stock_symbols.push(stock.stock_symbol.to_string());
                    let _ = writeln!(
                        &mut msg,
                        "    {} {} https://tw.stock.yahoo.com/quote/{}",
                        stock.name, stock.stock_symbol, stock.stock_symbol
                    );
                }
            }

            if let Err(why) = bot::telegram::send(&msg).await {
                logging::error_file_async(format!(
                    "Failed to telegram::send_to_allowed() because: {:?}",
                    why
                ));
            }

            //計算股利
            calculation::dividend_record::execute(year, Some(stock_symbols)).await;
        }
        Err(why) => {
            logging::error_file_async(format!("Failed to fetch StockEntity because: {:?}", why));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::logging;

    #[tokio::test]
    async fn test_calculate() {
        dotenv::dotenv().ok();
        logging::info_file_async("開始 execute".to_string());
        //let date = NaiveDate::from_ymd_opt(2023, 6, 15);
        //let today: NaiveDate = Local::today().naive_local();
        execute().await;

        logging::info_file_async("結束 execute".to_string());
    }
}
