use anyhow::Result;
use chrono::{Datelike, NaiveDate};

use crate::{
    database::{table, table::estimate::Estimate},
    logging,
};

/// 計算便宜、合理、昂貴價的估算
pub async fn calculate_estimated_price(date: NaiveDate) -> Result<()> {
    /*    let stocks = match SHARE.stocks.read() {
        Ok(stocks) => stocks.clone(),
        Err(why) => {
            return Err(anyhow!("Failed to read stocks cache because {:?}", why));
        }
    };*/

    let years: Vec<i32> = (0..10).map(|i| date.year() - i).collect();
    let years_str = years
        .iter()
        .map(|&year| year.to_string())
        .collect::<Vec<String>>()
        .join(",");
    /*let stock_symbols: Vec<String> = stocks.keys().cloned().collect();
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
    .await;*/

    Estimate::upsert_all(date, years_str).await?;

    let estimate_date_config = table::config::Config::new(
        "estimate-date".to_string(),
        date.format("%Y-%m-%d").to_string(),
    );

    estimate_date_config.set_val_as_naive_date().await?;
    logging::info_file_async("價格估值日期更新到資料庫完成".to_string());

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::cache::SHARE;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_calculate_estimated_price() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 calculate_estimated_price".to_string());
        let current_date = NaiveDate::parse_from_str("2023-09-15", "%Y-%m-%d").unwrap();
        match calculate_estimated_price(current_date).await {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to calculate_estimated_price because {:?}",
                    why
                ));
            }
        }
        logging::debug_file_async("結束 calculate_estimated_price".to_string());
    }
}
