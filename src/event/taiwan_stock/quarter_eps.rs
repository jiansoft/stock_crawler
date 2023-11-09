use std::collections::HashMap;

use anyhow::Result;
use chrono::{Datelike, Local};

use crate::{
    crawler::twse,
    database::{
        table::{
            self,
            stock::Stock
        }
    },
    declare::{Quarter, StockExchangeMarket},
    logging, util,
};

pub async fn execute() -> Result<()> {
    let now = Local::now();
    let current_quarter = Quarter::from_month(now.month()).unwrap();
    let previous_quarter = current_quarter.previous();
    let without_financial_stocks = table::stock::fetch_stocks_without_financial_statement(
        now.year(),
        previous_quarter.to_string().as_str(),
    )
    .await?;
    let without_financial_stocks = util::map::vec_to_hashmap(without_financial_stocks);
    for market in StockExchangeMarket::iterator() {
        if let Err(why) = process_eps(
            market,
            now.year(),
            previous_quarter,
            &without_financial_stocks,
        )
        .await
        {
            logging::error_file_async(format!(
                "Failed to update_suspend_listing because {:?}",
                why
            ));
        }
    }

    Ok(())
}

async fn process_eps(
    market: StockExchangeMarket,
    year: i32,
    quarter: Quarter,
    without_financial_stocks: &HashMap<String, Stock>,
) -> Result<()> {
    let eps = twse::eps::visit(market, year, quarter).await?;

    for e in eps {
        if !without_financial_stocks.contains_key(&e.stock_symbol) {
            //不在清單內代表已收錄數據
            continue;
        }

        let fs = table::financial_statement::FinancialStatement::from(e);

        if let Err(why) = fs.upsert_earnings_per_share().await {
            logging::error_file_async(format!("{:?}", why));
        }

        logging::debug_file_async(format!(
            "financial_statement earnings_per_share executed successfully. \r\n{:#?}",
            fs
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::cache::SHARE;

    use super::*;

    #[tokio::test]
    async fn test_process_eps() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::info_file_async("開始 process_eps".to_string());
        //let now = Local::now();
        let without_financial_stocks = table::stock::fetch_stocks_without_financial_statement(
            2018,
            Quarter::Q2.to_string().as_str(),
        )
        .await
        .unwrap();
        let without_financial_stocks = util::map::vec_to_hashmap(without_financial_stocks);
        //dbg!(without_financial_stocks);
        match process_eps(
            StockExchangeMarket::OverTheCounter,
            2018,
            Quarter::Q2,
            &without_financial_stocks,
        )
        .await
        {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!("Failed to process_eps because: {:?}", why));
            }
        }

        logging::info_file_async("結束 process_eps".to_string());
    }
}
