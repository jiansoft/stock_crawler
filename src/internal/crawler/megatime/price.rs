use std::collections::HashMap;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rust_decimal::Decimal;
use scraper::{Html, Selector};

use crate::{
    internal::{
        crawler::{
            megatime::{PcHome, HOST},
            StockInfo,
        },
    },
    util::{self, http::element}
};

//#stock_info_data_a > span.data_close
const SELECTOR: &str = r"#stock_info_data_a";

#[async_trait]
impl StockInfo for PcHome {
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        let url = format!(
            "https://{host}/stock/sid{symbol}.html",
            host = HOST,
            symbol = stock_symbol
        );
        let mut params = HashMap::new();
        params.insert("is_check", "1");
        let text = util::http::post(&url, None, Some(params)).await?;
        let document = Html::parse_document(&text);
        let selector = Selector::parse(SELECTOR)
            .map_err(|why| anyhow!("Failed to Selector::parse because: {:?}", why))?;

        if let Some(element) = document.select(&selector).next() {
            let price = element::parse_to_decimal(&element, "span.data_close");
            if price > Decimal::ZERO {
                return Ok(price);
            }
        }

        Err(anyhow!("Price element not found from pchome"))
    }
}

#[cfg(test)]
mod tests {
    use crate::logging;
    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 visit".to_string());

        match PcHome::get_stock_price("2330").await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("dividend : {:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because {:?}", why));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
    }
}
