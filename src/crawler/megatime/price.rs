//! # PCHome 股市爬蟲模組
//!
//! 此模組負責從 PCHome 股市 (pchome.megatime.com.tw) 抓取即時股票報價資訊。
//!
//! ## 主要功能
//! 1. **獲取即時股價**：取得單一股票的當前成交價。
//! 2. **獲取完整報價**：取得包含成交價、漲跌值與漲跌幅的完整 `StockQuotes` 資訊。

use std::collections::HashMap;
use std::str::FromStr;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use rust_decimal::Decimal;
use scraper::{Html, Selector};

use crate::{
    crawler::{
        megatime::{PcHome, HOST},
        StockInfo,
    },
    declare,
    util::{self, http::element, text},
};

/// 股票資訊容器的 CSS 選擇器（包含主 ID 與備援 Class）
static ROOT_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse("#stock_info_data_a, .price").expect("Failed to parse PCHome root selector")
});

#[async_trait]
impl StockInfo for PcHome {
    /// 取得指定股票代號的即時成交價。
    ///
    /// # 參數
    /// * `stock_symbol` - 股票代號（例如 "2330"）。
    ///
    /// # 回傳
    /// * `Ok(Decimal)` - 成功時回傳當前股價。
    /// * `Err` - 抓取失敗、解析錯誤或找不到該股票資料。
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        let (document, url) = Self::fetch_document(stock_symbol).await?;
        
        let root = document.select(&ROOT_SELECTOR).next()
            .ok_or_else(|| {
                let html_preview = document.html().chars().take(200).collect::<String>();
                anyhow!("在 {} 找不到股票 {} 的資訊容器。頁面開頭：{}", url, stock_symbol, html_preview)
            })?;

        let price = element::parse_to_decimal(&root, "span.data_close");
        if price > Decimal::ZERO {
            Ok(price.normalize())
        } else {
            Err(anyhow!("從 PCHome 解析到的股票 {} 價格為 0 或無效", stock_symbol))
        }
    }

    /// 取得指定股票代號的完整報價（價格、漲跌、漲跌幅）。
    ///
    /// # 參數
    /// * `stock_symbol` - 股票代號。
    ///
    /// # 回傳
    /// * `Ok(StockQuotes)` - 包含完整報價資訊的結構體。
    async fn get_stock_quotes(stock_symbol: &str) -> Result<declare::StockQuotes> {
        let (document, url) = Self::fetch_document(stock_symbol).await?;

        // 取得主要資訊容器
        let root = document.select(&ROOT_SELECTOR).next()
            .ok_or_else(|| {
                let body = document.html();
                let snippet = if body.len() > 500 { &body[0..500] } else { &body };
                anyhow!("在 {} 找不到股票 {} 的資訊容器 (#stock_info_data_a)。HTML 內容：\n{}", url, stock_symbol, snippet)
            })?;

        // 解析成交價
        let price_decimal = element::parse_to_decimal(&root, "span.data_close");
        if price_decimal == Decimal::ZERO {
            anyhow::bail!("無法解析股票 {} 的成交價", stock_symbol);
        }
        let price = f64::from_str(&price_decimal.to_string()).unwrap_or(0.0);

        // 解析漲跌值 (通常是第二個 span，包含漲跌符號 ▼/▲)
        let change_text = element::parse_value(&root, "span:nth-child(2)")
            .ok_or_else(|| anyhow!("無法解析股票 {} 的漲跌值", stock_symbol))?;
        let change = text::parse_f64(&change_text, Some(['▼', '▲'].to_vec()))?;

        // 解析漲跌幅 (通常是第三個 span)
        let range_text = element::parse_value(&root, "span:nth-child(3)")
            .ok_or_else(|| anyhow!("無法解析股票 {} 的漲跌幅", stock_symbol))?;
        let change_range = text::parse_f64(&range_text, None)?;

        Ok(declare::StockQuotes {
            stock_symbol: stock_symbol.to_string(),
            price,
            change,
            change_range,
        })
    }
}

impl PcHome {
    /// 私有輔助函式：發送 POST 請求並獲取解析後的 HTML 文件。
    /// 
    /// 該請求需要帶入 `is_check=1` 參數以獲取正確的報價內容。
    async fn fetch_document(stock_symbol: &str) -> Result<(Html, String)> {
        let url = format!(
            "https://{host}/stock/sid{symbol}.html",
            host = HOST,
            symbol = stock_symbol
        );
        
        let mut params = HashMap::new();
        params.insert("is_check", "1");
        
        let text = util::http::post(&url, None, Some(params))
            .await
            .with_context(|| format!("從 PCHome 獲取股票 {} 資料失敗 (URL: {})", stock_symbol, url))?;
            
        Ok((Html::parse_document(&text), url))
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
