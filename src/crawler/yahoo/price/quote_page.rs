//! # Yahoo 個股 Quote 頁解析
//!
//! 此模組保留原本以 Yahoo 個股頁 (`/quote/{symbol}`) 為基礎的解析邏輯，
//! 並在查詢前優先讀取 crawler 層維護的共用即時快取。
//! 這樣可以讓一般 `StockInfo` 呼叫端優先吃到 Yahoo 類股輪詢任務的成果，
//! 只有在快取未命中時才退回單檔頁面抓取。

use anyhow::Result;
use async_trait::async_trait;
use rust_decimal::Decimal;
use scraper::Html;

use crate::{
    cache::SHARE,
    crawler::{
        yahoo::{Yahoo, HOST},
        StockInfo,
    },
    declare,
    util::{self, text},
};

/// 將共用快取中的即時快照轉成 `StockQuotes` 回傳型別。
fn snapshot_to_quotes(
    snapshot: crate::cache::RealtimeSnapshot,
) -> Result<declare::StockQuotes> {
    snapshot.try_into_stock_quotes()
}

#[async_trait]
impl StockInfo for Yahoo {
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        // 先讀共用快取，因為開盤期間背景任務已經會持續刷新 Yahoo 類股資料。
        // 這樣一般查價請求就不必每次都打單檔頁，速度更快，也比較不容易被限流。
        if let Some(snapshot) = SHARE
            .get_stock_snapshot(stock_symbol)
            .filter(|snapshot| snapshot.price != Decimal::ZERO)
        {
            // 價格為 0 在這裡視為「缺值」而不是有效價格，
            // 所以只有非 0 才直接採信。
            return Ok(snapshot.price);
        }

        // 快取沒命中時才退回 Yahoo 單檔 quote 頁，
        // 這條路徑是保底邏輯，確保背景任務尚未暖機時仍能查到資料。
        let url = format!(
            "https://{host}/quote/{symbol}",
            host = HOST,
            symbol = stock_symbol
        );
        // 先抓原始 HTML，再交給 scraper 做結構化查找。
        let text = util::http::get(&url, None).await?;
        let document = Html::parse_document(&text);

        // 這裡沿用既有 Yahoo quote 頁的 selector，
        // 只抓主要報價區塊中的大字成交價。
        let price_str =
            util::http::element::get_one_element(util::http::element::GetOneElementText {
                stock_symbol,
                document,
                selector: "#main-0-QuoteHeader-Proxy",
                element: "span.Fz\\(32px\\)",
                url: &url,
            })?;

        // 解析完後 normalize，避免 `Decimal` 保留不必要的小數 scale，
        // 讓後續比對與 log 看起來更乾淨。
        Ok(text::parse_decimal(&price_str, None)?.normalize())
    }

    async fn get_stock_quotes(stock_symbol: &str) -> Result<declare::StockQuotes> {
        // `get_stock_quotes` 比 `get_stock_price` 多了漲跌與漲跌幅，
        // 但優先吃快取的策略相同，先減少對單檔頁的依賴。
        if let Some(snapshot) = SHARE
            .get_stock_snapshot(stock_symbol)
            .filter(|snapshot| snapshot.price != Decimal::ZERO)
        {
            // 命中快取時直接轉成回傳型別，不走 HTML 解析。
            return snapshot_to_quotes(snapshot);
        }

        // 以下才是 fallback 路徑：真的需要時才打 Yahoo 單檔頁。
        let url = format!(
            "https://{host}/quote/{symbol}",
            host = HOST,
            symbol = stock_symbol
        );
        let text = util::http::get(&url, None).await?;
        let document = Html::parse_document(&text);

        // Yahoo 的漲跌顏色有時比純文字更可靠，
        // 所以先檢查是否存在「下跌」顏色 class，後面再用來校正正負號。
        let is_negative = document
            .select(
                &scraper::Selector::parse("#main-0-QuoteHeader-Proxy .C\\(\\$c-trend-down\\)")
                    .unwrap(),
            )
            .next()
            .is_some();

        // 成交價、漲跌與漲跌幅分三次抓，雖然看起來重複，
        // 但能讓錯誤訊息明確落在哪一個欄位 selector。
        let price_raw =
            util::http::element::get_one_element(util::http::element::GetOneElementText {
                stock_symbol,
                document: document.clone(),
                selector: "#main-0-QuoteHeader-Proxy",
                element: "span.Fz\\(32px\\)",
                url: &url,
            })?;
        let price = text::parse_f64(&price_raw, None)?;

        // 漲跌值使用較小字級的數字節點。
        let change_raw =
            util::http::element::get_one_element(util::http::element::GetOneElementText {
                stock_symbol,
                document: document.clone(),
                selector: "#main-0-QuoteHeader-Proxy",
                element: "span.Fz\\(20px\\)",
                url: &url,
            })?;
        let mut change = text::parse_f64(&change_raw, None)?;

        // 漲跌幅通常包在括號與百分號中，所以 parse 時一併去掉。
        let range_raw =
            util::http::element::get_one_element(util::http::element::GetOneElementText {
                stock_symbol,
                document,
                selector: "#main-0-QuoteHeader-Proxy",
                element: "span.Jc\\(fe\\)",
                url: &url,
            })?;
        let mut change_range = text::parse_f64(&range_raw, Some(vec!['(', ')', '%']))?;

        // Yahoo 頁面上的數字有時是正數文字，但實際方向靠顏色表現，
        // 所以這裡用 `is_negative` 再校正一次，避免 +/− 號判錯。
        if is_negative {
            if change > 0.0 {
                change = -change;
            }
            if change_range > 0.0 {
                change_range = -change_range;
            }
        }

        // 最後統一包回專案內通用的報價型別，讓外部呼叫端不需要知道 Yahoo 頁面結構。
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

    /// Live 測試：驗證單檔 Yahoo quote 頁仍可抓到指定股票的成交價。
    #[tokio::test]
    #[ignore]
    async fn test_get_stock_price() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 visit".to_string());

        match Yahoo::get_stock_price("2330").await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("yahoo::get_stock_price {:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because {:?}", why));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
    }

    /// Live 測試：驗證單檔 Yahoo quote 頁仍可抓到成交價、漲跌與漲幅資訊。
    #[tokio::test]
    #[ignore]
    async fn test_get_stock_quotes() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 yahoo::get_stock_quotes".to_string());

        match Yahoo::get_stock_quotes("2330").await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("yahoo::get_stock_quotes {:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to yahoo::get_stock_quotes because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 yahoo::get_stock_quotes".to_string());
    }
}
