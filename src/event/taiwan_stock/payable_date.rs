use std::fmt::Write;

use anyhow::Result;
use chrono::{Local, NaiveDate};

use crate::{
    bot::{self, telegram::Telegram},
    database::table::dividend,
};

/// 提提醒本日發放股利的股票(只通知自已有的股票)
pub async fn execute() -> Result<()> {
    let today: NaiveDate = Local::now().date_naive();
    let stocks_payable_date_info =
        dividend::extension::stock_dividend_payable_date_info::fetch(today).await?;
    if stocks_payable_date_info.is_empty() {
        return Ok(());
    }

    let mut stock_symbols: Vec<String> = Vec::with_capacity(stocks_payable_date_info.len());
    let mut msg = String::with_capacity(2048);

    if writeln!(
        &mut msg,
        "{} 進行股利發放的股票如下︰",
        Telegram::escape_markdown_v2(today.to_string())
    )
    .is_ok()
    {
        for stock in stocks_payable_date_info {
            stock_symbols.push(stock.stock_symbol.to_string());
            let _ = write!(
                &mut msg,
                "    {0} {1} ",
                stock.stock_symbol,
                Telegram::escape_markdown_v2(stock.name),
            );

            if stock.payable_date1 != "-" {
                let _ = write!(
                    &mut msg,
                    "現金︰{0}元 ",
                    Telegram::escape_markdown_v2(stock.cash_dividend.normalize().to_string()),
                );
            }

            if stock.payable_date2 != "-" {
                let _ = write!(
                    &mut msg,
                    "股票︰{0}元 ",
                    Telegram::escape_markdown_v2(stock.stock_dividend.normalize().to_string()),
                );
            }

            let _ = writeln!(
                &mut msg,
                "合計︰{0}元 ",
                Telegram::escape_markdown_v2(stock.sum.normalize().to_string()),
            );
        }
    }

    //群內通知
    bot::telegram::send(&msg).await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_calculate() {
        dotenv::dotenv().ok();
        logging::info_file_async("開始 execute".to_string());
        //let date = NaiveDate::from_ymd_opt(2023, 6, 15);
        //let today: NaiveDate = Local::today().naive_local();
        let _ = execute().await;

        logging::info_file_async("結束 execute".to_string());
    }
}
