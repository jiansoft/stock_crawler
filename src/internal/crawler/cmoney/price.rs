use anyhow::Result;
use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::{
    internal::{
        crawler::{
            cmoney::{CMoney, HOST},
            StockInfo,
        },
    },
    util
};

#[async_trait]
impl StockInfo for CMoney {
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        let target = util::http::element::GetOneElementText {
            stock_symbol,
            url: &format!(
                "https://{host}/forum/stock/{symbol}",
                host = HOST,
                symbol = stock_symbol
            ),
            selector: "section > div",
            element: "div.stockData__info > div",
        };

        util::http::element::get_one_element_as_decimal(target).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 visit".to_string());

        match CMoney::get_stock_price("3008").await {
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
