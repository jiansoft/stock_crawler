use anyhow::Result;
use async_trait::async_trait;
use rust_decimal::Decimal;
use scraper::Html;

use crate::{
    crawler::{
        cmoney::{CMoney, HOST},
        StockInfo,
    },
    declare,
    util::{self, text},
};

/// CMoney 即時報價抓取實作。
///
/// 此實作會抓取 CMoney 個股頁面，解析當前股價與漲跌資訊。
#[async_trait]
impl StockInfo for CMoney {
    /// 取得單一股票的即時價格。
    ///
    /// 會回傳解析後的十進位價格；若網頁結構或內容異常則回傳錯誤。
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        let url = format!(
            "https://{host}/forum/stock/{symbol}",
            host = HOST,
            symbol = stock_symbol
        );
        let text = util::http::get(&url, None).await?;
        let document = Html::parse_document(&text);
        let target = util::http::element::GetOneElementText {
            stock_symbol,
            url: &url,
            selector: "section > div",
            element: "div.stockData__info > div",
            document,
        };

        util::http::element::get_one_element_as_decimal(target)
    }

    /// 取得單一股票的即時報價資訊。
    ///
    /// 包含目前價格、漲跌價差與漲跌幅百分比。
    async fn get_stock_quotes(stock_symbol: &str) -> Result<declare::StockQuotes> {
        let url = &format!(
            "https://{host}/forum/stock/{symbol}",
            host = HOST,
            symbol = stock_symbol
        );
        let text = util::http::get(url, None).await?;
        let document = Html::parse_document(&text);

        let price = util::http::element::get_one_element(util::http::element::GetOneElementText {
            stock_symbol,
            url,
            selector: "section > div",
            element: "div.stockData__info > div",
            document: document.clone(),
        })?;
        let price = text::parse_f64(&price, None)?;

        let change =
            util::http::element::get_one_element(util::http::element::GetOneElementText {
                stock_symbol,
                url,
                selector: r"section > div",
                element: r"div.stockData__info > div.stockData__value > div.stockData__quotePrice",
                document: document.clone(),
            })?;
        let change = text::parse_f64(&change, None)?;

        let change_range =
            util::http::element::get_one_element(util::http::element::GetOneElementText {
                stock_symbol,
                url,
                selector: r"section > div",
                element: r"div.stockData__info > div.stockData__value > div.stockData__quote",
                document: document.clone(),
            })?;
        let change_range = text::parse_f64(&change_range, Some(['(', ')'].to_vec()))?;

        Ok(declare::StockQuotes {
            stock_symbol: stock_symbol.to_string(),
            price,
            change,
            change_range,
        })
    }
}

#[cfg(test)]
/// CMoney 報價抓取相關測試。
///
/// 這些測試需連線外部網站，執行結果會受網路與來源頁面變動影響。
mod tests {
    use super::*;
    use crate::logging;

    #[tokio::test]
    /// 測試可取得指定股票即時價格。
    async fn test_get_stock_price() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 get_stock_price".to_string());

        match CMoney::get_stock_price("3008").await {
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
    /// 測試可取得指定股票完整即時報價。
    async fn test_get_stock_quotes() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 get_stock_quotes".to_string());

        match CMoney::get_stock_quotes("6792").await {
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
