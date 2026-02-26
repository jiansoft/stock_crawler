//! # Yahoo 股價採集器
//!
//! 此模組實作了 `StockInfo` Trait，負責從 Yahoo 財經抓取股票的最新成交價與詳細報價。
//!
//! ## 選擇器說明
//!
//! Yahoo 台灣站的 HTML 使用了 CSS Modules 或類似的混淆技術，因此選擇器依賴於特定的大字體 Class：
//! - `span.Fz(32px)`：大號字體，通常代表最新成交價。
//! - `span.Fz(20px)`：中號字體，通常代表漲跌價。
//! - `span.Jc(fe)`：右對齊容器，包含漲跌幅百分比。
//!
//! ## 錯誤處理機制
//!
//! 如果頁面結構發生變化導致無法找到關鍵標籤，函數將回傳 `Err` 而非預設值。
//! 這能觸發 `crawler/mod.rs` 中的自動重試機制，嘗試從其他站點（如 CMoney）獲取資料。

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
    /// 獲取指定股票的最新成交價。
    ///
    /// # 參數
    /// * `stock_symbol` - 股票代碼 (例如: "2330")
    ///
    /// # 實作細節
    /// 解析頁面中的 `.Fz(32px)` 標籤並將其標準化為 `Decimal` 型態。
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        let url = format!("https://{host}/quote/{symbol}", host = HOST, symbol = stock_symbol);
        let text = util::http::get(&url, None).await?;
        let document = Html::parse_document(&text);

        let price_str = util::http::element::get_one_element(util::http::element::GetOneElementText {
            stock_symbol,
            document,
            selector: "#main-0-QuoteHeader-Proxy",
            element: "span.Fz\\(32px\\)",
            url: &url,
        })?;

        Ok(text::parse_decimal(&price_str, None)?.normalize())
    }

    /// 獲取指定股票的完整報價資訊（包含漲跌、漲幅）。
    ///
    /// # 參數
    /// * `stock_symbol` - 股票代碼 (例如: "2330")
    ///
    /// # 實作細節
    /// 此函數會解析成交價、漲跌價與漲幅百分比。
    /// 並透過檢查是否有 `.C($c-trend-down)` 顏色類別來判定趨勢是否為下跌，
    /// 藉此修正部分頁面未明確標註負號的情況。
    async fn get_stock_quotes(stock_symbol: &str) -> Result<declare::StockQuotes> {
        let url = format!("https://{host}/quote/{symbol}", host = HOST, symbol = stock_symbol);
        let text = util::http::get(&url, None).await?;
        let document = Html::parse_document(&text);

        // 檢查是否下跌：尋找帶有跌幅顏色特徵的趨勢 Class
        let is_negative = document
            .select(&scraper::Selector::parse("#main-0-QuoteHeader-Proxy .C\\(\\$c-trend-down\\)").unwrap())
            .next()
            .is_some();

        // 取得成交價 (Fz(32px))
        let price_raw = util::http::element::get_one_element(util::http::element::GetOneElementText {
            stock_symbol,
            document: document.clone(),
            selector: "#main-0-QuoteHeader-Proxy",
            element: "span.Fz\\(32px\\)",
            url: &url,
        })?;
        let price = text::parse_f64(&price_raw, None)?;

        // 取得漲跌 (Fz(20px))
        let change_raw = util::http::element::get_one_element(util::http::element::GetOneElementText {
            stock_symbol,
            document: document.clone(),
            selector: "#main-0-QuoteHeader-Proxy",
            element: "span.Fz\\(20px\\)",
            url: &url,
        })?;
        let mut change = text::parse_f64(&change_raw, None)?;

        // 取得漲幅百分比 (Jc(fe))
        let range_raw = util::http::element::get_one_element(util::http::element::GetOneElementText {
            stock_symbol,
            document,
            selector: "#main-0-QuoteHeader-Proxy",
            element: "span.Jc\\(fe\\)",
            url: &url,
        })?;
        let mut change_range = text::parse_f64(&range_raw, Some(vec!['(', ')', '%']))?;

        // 防禦性修正：Yahoo 網頁上的數值有時不帶符號，根據顏色類別進行強制校正
        if is_negative {
            if change > 0.0 { change = -change; }
            if change_range > 0.0 { change_range = -change_range; }
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

        match Yahoo::get_stock_quotes("2330").await {
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
