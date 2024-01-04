use anyhow::Result;
use async_trait::async_trait;
use rust_decimal::Decimal;
use scraper::Html;

use crate::{
    crawler::{
        histock::{HiStock, HOST},
        StockInfo,
    },
    declare,
    util::{self, text},
};

#[async_trait]
impl StockInfo for HiStock {
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        let url = format!(
            "https://{host}/stock/{symbol}",
            host = HOST,
            symbol = stock_symbol
        );
        let text = util::http::get(&url, None).await?;
        let document = Html::parse_document(&text);
        let target = util::http::element::GetOneElementText {
            stock_symbol,
            url: &url,
            selector: "#Price1_lbTPrice",
            element: "span",
            document,
        };

        util::http::element::get_one_element_as_decimal(target)
    }

    async fn get_stock_quotes(stock_symbol: &str) -> Result<declare::StockQuotes> {
        let url = &format!(
            "https://{host}/stock/{symbol}",
            host = HOST,
            symbol = stock_symbol
        );
        let text = util::http::get(url, None).await?;
        let document = Html::parse_document(&text);
        let price = util::http::element::get_one_element(util::http::element::GetOneElementText {
            stock_symbol,
            url,
            selector: "#Price1_lbTPrice",
            element: "span",
            document: document.clone(),
        })?;
        let price = text::parse_f64(&price, None)?;
        let change =
            util::http::element::get_one_element(util::http::element::GetOneElementText {
                stock_symbol,
                url,
                selector: r"#Price1_lbTChange",
                element: r"span",
                document: document.clone(),
            })?;
        let is_negative = change.contains('▼');
        let mut change = text::parse_f64(&change, Some(['▼', '▲'].to_vec()))?;
        let change_range =
            util::http::element::get_one_element(util::http::element::GetOneElementText {
                stock_symbol,
                url,
                selector: r"#Price1_lbTPercent",
                element: r"span",
                document: document.clone(),
            })?;
        let change_range = text::parse_f64(&change_range, None)?;

        if is_negative {
            change = -change;
        }

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

        match HiStock::get_stock_price("3008").await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("price : {:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to get_stock_price because {:?}", why));
            }
        }

        logging::debug_file_async("結束 get_stock_price".to_string());
    }

    #[tokio::test]
    async fn test_get_stock_quotes() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 get_stock_quotes".to_string());

        match HiStock::get_stock_quotes("2888").await {
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
