use std::collections::HashMap;

use anyhow::Result;
use chrono::{Datelike, Local, TimeDelta};

use crate::{
    crawler::twse,
    database::{
        table::{
            self,
            stock::Stock,
            financial_statement
        }
    },
    declare::{Quarter, StockExchangeMarket},
    logging,
    util,
};

pub async fn execute() -> Result<()> {
    let now = Local::now();
    let previous_quarter = now - TimeDelta::try_days(130).unwrap();
    let year = previous_quarter.year();
    let previous_quarter = Quarter::from_month(now.month()).unwrap().previous();
    let quarter = previous_quarter.to_string();
    let without_fs_stocks = table::stock::fetch_stocks_without_financial_statement(
        year,
        quarter.to_string().as_str(),
    )
    .await?;
    let without_financial_stocks = util::map::vec_to_hashmap(without_fs_stocks);

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
            continue;
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

    for mut e in eps {
        if !without_financial_stocks.contains_key(&e.stock_symbol) {
            //不在清單內代表已收錄數據
            continue;
        }

        if e.quarter != Quarter::Q1 {
            //如果不是第一季的EPS要減掉今年其他的EPS，例如Q2要減 Q1，Q3要減Q2、Q1
            let smaller_quarters = quarter.smaller_quarters();
            let before_eps = financial_statement::fetch_cumulative_eps(&e.stock_symbol, year, smaller_quarters).await?;
            e.earnings_per_share -= before_eps;
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
    use std::time::Duration;
    use crate::cache::SHARE;

    use super::*;

    #[tokio::test]
    async fn test_process_eps() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::info_file_async("開始 process_eps".to_string());
        //let now = Local::now();
        let without_financial_stocks = table::stock::fetch_stocks_without_financial_statement(
            2024,
            Quarter::Q1.to_string().as_str(),
        )
        .await
        .unwrap();
        let without_financial_stocks = util::map::vec_to_hashmap(without_financial_stocks);
        //dbg!(without_financial_stocks);
        match process_eps(
            StockExchangeMarket::Listed,
            2023,
            Quarter::Q4,
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
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
