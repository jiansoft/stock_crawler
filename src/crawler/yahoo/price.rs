use anyhow::Result;
use async_trait::async_trait;
use rust_decimal::Decimal;
use scraper::Html;

use crate::{
    crawler::{
        yahoo::{Yahoo, HOST},
        StockInfo,
    },
    declare,
    util::{self, text},
};

#[async_trait]
impl StockInfo for Yahoo {
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        let url = &format!(
            "https://{host}/quote/{symbol}",
            host = HOST,
            symbol = stock_symbol
        );
        let text = util::http::get(url, None).await?;
        let document = Html::parse_document(&text);
        let price = util::http::element::get_one_element(util::http::element::GetOneElementText {
            stock_symbol,
            document: document.clone(),
            selector: "#main-0-QuoteHeader-Proxy > div > div > div > div",
            element: "span",
            url,
        })?;

        match text::parse_decimal(&price, None) {
            Ok(p) => Ok(p.normalize()),
            Err(_) => Ok(Decimal::ZERO),
        }
    }

    async fn get_stock_quotes(stock_symbol: &str) -> Result<declare::StockQuotes> {
        let url = &format!(
            "https://{host}/quote/{symbol}",
            host = HOST,
            symbol = stock_symbol
        );
        let text = util::http::get(url, None).await?;
        let document = Html::parse_document(&text);

        // 下跌
        //  > span
        //#main-0-QuoteHeader-Proxy > div > div.D\(f\).Jc\(sb\).Ai\(fe\) > div.D\(f\).Fld\(c\).Ai\(fs\) > div > span.Fz\(20px\).Fw\(b\).Lh\(1\.2\).Mend\(4px\).D\(f\).Ai\(c\).C\(\$c-trend-up\) > span
        //Negative

        let is_negative = util::http::element::get_one_element(
            util::http::element::GetOneElementText {
                stock_symbol,
                document: document.clone(),
                selector: r"#main-0-QuoteHeader-Proxy > div > div > div > div > span.Fz\(20px\).Fw\(b\).Lh\(1\.2\).Mend\(4px\).D\(f\).Ai\(c\).C\(\$c-trend-down\)",
                element: "span",
                url,
            },
        ).is_ok();
        let price = util::http::element::get_one_element(util::http::element::GetOneElementText {
            stock_symbol,
            document: document.clone(),
            selector: "#main-0-QuoteHeader-Proxy > div > div > div > div",
            element: "span",
            url,
        })?;
        let price = text::parse_f64(&price, None)?;
        let change =
            util::http::element::get_one_element(util::http::element::GetOneElementText {
                stock_symbol,
                url,
                selector: r"#main-0-QuoteHeader-Proxy > div > div > div > div",
                element: r"span.Fz\(20px\).Fw\(b\).Lh\(1\.2\).Mend\(4px\).D\(f\)",
                document: document.clone(),
            })?;

        let mut change = text::parse_f64(&change, None)?;
        let change_range =
            util::http::element::get_one_element(util::http::element::GetOneElementText {
                stock_symbol,
                url,
                selector: r"#main-0-QuoteHeader-Proxy > div > div > div > div",
                element: r"span.Jc\(fe\)",
                document: document.clone(),
            })?;
        let mut change_range = text::parse_f64(&change_range, Some(['(', ')'].to_vec()))?;

        if is_negative {
            change = -change;
            change_range = -change_range;
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
    async fn test_visit() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 visit".to_string());

        match Yahoo::get_stock_price("2330").await {
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

    #[tokio::test]
    async fn test_get_stock_quotes() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 get_stock_quotes".to_string());

        match Yahoo::get_stock_quotes("2364").await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("get_stock_quotes : {:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to get_stock_quotes because {:?}", why));
            }
        }

        logging::debug_file_async("結束 get_stock_quotes".to_string());
    }
}
