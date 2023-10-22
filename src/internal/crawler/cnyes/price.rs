use anyhow::Result;
use rust_decimal::Decimal;

use crate::internal::{crawler::cnyes::HOST, util};

pub async fn get(stock_symbol: &str) -> Result<Decimal> {
    let target = util::http::element::GetOneElementText {
        stock_symbol,
        url: &format!(
            "https://{host}/twstock/{symbol}",
            host = HOST,
            symbol = stock_symbol
        ),
        selector: "div.jsx-162737614.container > div",
        element: "h3",
    };

    util::http::element::get_one_element_as_decimal(target).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::logging;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 visit".to_string());

        match get("2330").await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("price : {:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because {:?}", why));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
    }
}
