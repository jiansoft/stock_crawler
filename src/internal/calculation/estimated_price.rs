use anyhow::{anyhow, Result};
use chrono::{Datelike, NaiveDate};
use futures::{stream, StreamExt};

use crate::internal::{cache::SHARE, database::table::estimate::Estimate, logging, util};

pub async fn calculate_estimated_price(date: NaiveDate) -> Result<()> {
    let stocks = match SHARE.stocks.read() {
        Ok(stocks) => stocks.clone(),
        Err(why) => {
            return Err(anyhow!("Failed to read stocks cache because {:?}", why));
        }
    };

    let years: Vec<i32> = (0..10).map(|i| date.year() - i).collect();
    let years_vec: Vec<String> = years.iter().map(|&year| year.to_string()).collect();
    let years_str = years_vec.join(",");
    let stock_symbols: Vec<String> = stocks.keys().cloned().collect();

    stream::iter(stock_symbols)
        .for_each_concurrent(util::concurrent_limit_32(), |stock_symbol| {
            let years = years_str.clone();
            async move {
                let estimate = Estimate::new(stock_symbol, date);
                if let Err(why) = estimate.upsert(years).await {
                    logging::error_file_async(format!("{:?}", why));
                }
            }
        })
        .await;

    Ok(())
}
