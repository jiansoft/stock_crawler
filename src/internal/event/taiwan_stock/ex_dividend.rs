use std::fmt::Write;

use anyhow::Result;
use chrono::{Datelike, Local, NaiveDate};

use crate::internal::{bot, calculation, database::table::dividend};

/// 提醒本日為除權息的股票有那些
pub async fn execute() -> Result<()> {
    let today: NaiveDate = Local::now().date_naive();
    let stocks_dividend_info =
        dividend::extension::stock_dividend_info::fetch_stocks_with_dividends_on_date(today)
            .await?;
    if stocks_dividend_info.is_empty() {
        return Ok(());
    }

    let mut stock_symbols: Vec<String> = Vec::with_capacity(stocks_dividend_info.len());
    let mut msg = String::with_capacity(2048);
    if writeln!(&mut msg, "{} 進行除權息的股票如下︰", today).is_ok() {
        for stock in stocks_dividend_info {
            stock_symbols.push(stock.stock_symbol.to_string());
            let _ = writeln!(
                &mut msg,
                "    {0} {1} 現金︰{2}元 股票 {3}元 合計︰{4}元 https://tw.stock.yahoo.com/quote/{0}",
                stock.stock_symbol, stock.name, stock.cash_dividend.normalize(), stock.stock_dividend.normalize(), stock.sum.normalize()
            );
        }
    }

    //計算股利
    calculation::dividend_record::execute(today.year(), Some(stock_symbols)).await;
    //群內通知
    bot::telegram::send(&msg).await
}

#[cfg(test)]
mod tests {
    use crate::internal::logging;

    use super::*;

    #[tokio::test]
    async fn test_calculate() {
        dotenv::dotenv().ok();
        logging::info_file_async("開始 execute".to_string());
        //let date = NaiveDate::from_ymd_opt(2023, 6, 15);
        //let today: NaiveDate = Local::today().naive_local();
        let _ = execute().await;

        logging::info_file_async("結束 execute".to_string());
    }
}
