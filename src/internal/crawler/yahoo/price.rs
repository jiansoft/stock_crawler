use anyhow::{anyhow, Result};
use rust_decimal::Decimal;
use scraper::{Html, Selector};

use crate::internal::{
    crawler::yahoo::HOST,
    logging,
    util::{self, http::element},
};

const SELECTOR: &str = r"#main-0-QuoteHeader-Proxy > div > div > div > div";

pub async fn get(stock_symbol: &str) -> Result<Decimal> {
    let url = format!("https://{}/quote/{}", HOST, stock_symbol);
    logging::info_file_async(format!("visit url:{}", url));
    let text = util::http::get(&url, None).await?;
    let document = Html::parse_document(&text);
    let selector = Selector::parse(SELECTOR)
        .map_err(|why| anyhow!("Failed to Selector::parse because: {:?}", why))?;

    if let Some(element) = document.select(&selector).next() {
        let price = element::parse_to_decimal(&element, "span");
        if price > Decimal::ZERO {
            logging::debug_file_async(format!("{} price : {:#?} from yahoo", stock_symbol, price));
            return Ok(price);
        }
    }

    Err(anyhow!("Price element not found from yahoo"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 visit".to_string());

        match get("2330").await {
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
