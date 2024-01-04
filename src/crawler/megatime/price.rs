use std::collections::HashMap;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rust_decimal::Decimal;
use scraper::{Html, Selector};

use crate::{
    util::{
        text,
        self,
        http::element
    },
    crawler::{
        megatime::{PcHome, HOST},
        StockInfo,
    },
    declare,
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

    async fn get_stock_quotes(stock_symbol: &str) -> Result<declare::StockQuotes> {
        let url = &format!(
            "https://{host}/stock/sid{symbol}.html",
            host = HOST,
            symbol = stock_symbol
        );
        let mut params = HashMap::new();
        params.insert("is_check", "1");
        let text = util::http::post(url, None, Some(params)).await?;
        let document = Html::parse_document(&text);

        let price = element::get_one_element(util::http::element::GetOneElementText {
            stock_symbol,
            url,
            selector: "#stock_info_data_a",
            element: "span.data_close",
            document: document.clone(),
        })?;
        let price = text::parse_f64(&price, None)?;

        let change =
            util::http::element::get_one_element(util::http::element::GetOneElementText {
                stock_symbol,
                url,
                selector: r"#stock_info_data_a",
                element: r"span:nth-child(2)",
                document: document.clone(),
            })?;
        let change = text::parse_f64(&change, Some(['▼', '▲'].to_vec()))?;

        let change_range =
            util::http::element::get_one_element(util::http::element::GetOneElementText {
                stock_symbol,
                url,
                selector: r"#stock_info_data_a",
                element: r"span:nth-child(3)",
                document: document.clone(),
            })?;
        let change_range = text::parse_f64(&change_range, None)?;

        Ok(declare::StockQuotes {
            stock_symbol: stock_symbol.to_string(),
            price,
            change,
            change_range,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging;

    #[tokio::test]
    async fn test_get_stock_price() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 get_stock_price".to_string());

        match PcHome::get_stock_price("2330").await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("price : {:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to get_stock_price because {:?}", why));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
    }

    #[tokio::test]
    async fn test_get_stock_quotes() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 get_stock_quotes".to_string());

        match PcHome::get_stock_quotes("2330").await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("get_stock_quotes : {:#?}", e));
            }
            Err(why) => {
                dbg!(&why);
                logging::debug_file_async(format!("Failed to get_stock_quotes because {:?}", why));
            }
        }

        logging::debug_file_async("結束 get_stock_quotes".to_string());
    }
}
