//! # PCHome 股市爬蟲模組
//!
//! 此模組負責從 PCHome 股市 (pchome.megatime.com.tw) 抓取即時股票報價資訊。
//!
//! ## 主要功能
//! 1. **獲取即時股價**：取得單一股票的當前成交價。
//! 2. **獲取完整報價**：取得包含成交價、漲跌值與漲跌幅的完整 `StockQuotes` 資訊。

use std::collections::HashMap;
use std::str::FromStr;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use rust_decimal::Decimal;
use scraper::{Html, Selector};

use crate::{
    core::declare,
    core::util::{self, http::element, text},
    infra::crawler::{
        StockInfo,
        megatime::{HOST, PcHome},
    },
};

/// 股票資訊容器的 CSS 選擇器（包含主 ID 與備援 Class）
static ROOT_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse("#stock_info_data_a, .price").expect("Failed to parse PCHome root selector")
});

/// 解析 PCHome 個股頁 HTML 中的即時成交價。
///
/// 這是一個「純函式」——輸入只有 HTML 字串，不做任何網路 I/O，
/// 可用 `testdata/stock_page.html` fixture 直接驗證（fetch/parse 分離）。
fn parse_stock_price_html(stock_symbol: &str, url: &str, html: &str) -> Result<Decimal> {
    let document = Html::parse_document(html);
    let root = document.select(&ROOT_SELECTOR).next().ok_or_else(|| {
        let html_preview = document.html().chars().take(200).collect::<String>();
        anyhow!(
            "在 {} 找不到股票 {} 的資訊容器。頁面開頭：{}",
            url,
            stock_symbol,
            html_preview
        )
    })?;

    let price = element::parse_to_decimal(&root, "span.data_close");
    if price > Decimal::ZERO {
        Ok(price.normalize())
    } else {
        Err(anyhow!(
            "從 PCHome 解析到的股票 {} 價格為 0 或無效",
            stock_symbol
        ))
    }
}

/// 解析 PCHome 個股頁 HTML 中的完整報價（價格、漲跌、漲跌幅）。
///
/// 與 [`parse_stock_price_html`] 一樣是純函式。
/// 漲跌值帶方向符號（▼/▲），解析時去除符號但保留負號。
fn parse_stock_quotes_html(
    stock_symbol: &str,
    url: &str,
    html: &str,
) -> Result<declare::StockQuotes> {
    let document = Html::parse_document(html);

    // 取得主要資訊容器
    let root = document.select(&ROOT_SELECTOR).next().ok_or_else(|| {
        let body = document.html();
        // 錯誤訊息只需要頁面開頭片段。用 text::truncate 依「字元」截斷，
        // 而不是 &body[0..500] 這種 byte index 切片——PCHome 頁面含中文
        //（每字 3 bytes），byte 500 若落在字元中間會直接 panic。
        let snippet = text::truncate(&body, 500);
        anyhow!(
            "在 {} 找不到股票 {} 的資訊容器 (#stock_info_data_a)。HTML 內容：\n{}",
            url,
            stock_symbol,
            snippet
        )
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

#[async_trait]
impl StockInfo for PcHome {
    /// 取得指定股票代號的即時成交價。
    ///
    /// 只負責 HTTP 抓取，解析交給 [`parse_stock_price_html`]。
    ///
    /// # 參數
    /// * `stock_symbol` - 股票代號（例如 "2330"）。
    ///
    /// # 回傳
    /// * `Ok(Decimal)` - 成功時回傳當前股價。
    /// * `Err` - 抓取失敗、解析錯誤或找不到該股票資料。
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        let (text, url) = Self::fetch_page(stock_symbol).await?;
        parse_stock_price_html(stock_symbol, &url, &text)
    }

    /// 取得指定股票代號的完整報價（價格、漲跌、漲跌幅）。
    ///
    /// 只負責 HTTP 抓取，解析交給 [`parse_stock_quotes_html`]。
    ///
    /// # 參數
    /// * `stock_symbol` - 股票代號。
    ///
    /// # 回傳
    /// * `Ok(StockQuotes)` - 包含完整報價資訊的結構體。
    async fn get_stock_quotes(stock_symbol: &str) -> Result<declare::StockQuotes> {
        let (text, url) = Self::fetch_page(stock_symbol).await?;
        parse_stock_quotes_html(stock_symbol, &url, &text)
    }
}

impl PcHome {
    /// 私有輔助函式：發送 POST 請求並取得原始 HTML 字串。
    ///
    /// 該請求需要帶入 `is_check=1` 參數以獲取正確的報價內容。
    /// 回傳字串而非 `Html`，讓解析工作留在純函式（fetch/parse 分離）。
    async fn fetch_page(stock_symbol: &str) -> Result<(String, String)> {
        let url = format!(
            "https://{host}/stock/sid{symbol}.html",
            host = HOST,
            symbol = stock_symbol
        );

        let mut params = HashMap::new();
        params.insert("is_check", "1");

        let text = util::http::post(&url, None, Some(params))
            .await
            .with_context(|| {
                format!(
                    "從 PCHome 獲取股票 {} 資料失敗 (URL: {})",
                    stock_symbol, url
                )
            })?;

        Ok((text, url))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::crawler::log_stock_price_test;
    use rust_decimal_macros::dec;

    /// 以貼近真實個股頁形狀的 fixture 驗證整頁解析流程（下跌情境）。
    #[test]
    fn parse_stock_page_fixture_extracts_price_and_quotes() {
        // include_str! 的路徑相對於本檔案（megatime/price.rs）→ megatime/testdata/。
        const FIXTURE: &str = include_str!("testdata/stock_page.html");
        let url = "https://pchome.megatime.com.tw/stock/sid2884.html";

        let price = parse_stock_price_html("2884", url, FIXTURE).unwrap();
        assert_eq!(price, dec!(173.5));

        let quotes = parse_stock_quotes_html("2884", url, FIXTURE).unwrap();
        assert_eq!(quotes.stock_symbol, "2884");
        assert_eq!(quotes.price, 173.5);
        // 漲跌原文是「▼-2.5」：去掉方向符號、保留負號。
        assert_eq!(quotes.change, -2.5);
        // 漲跌幅原文是「-1.42%」：% 由預設清單去除。
        assert_eq!(quotes.change_range, -1.42);
    }

    /// 頁面缺少資訊容器（改版或載到錯誤頁）時必須明確報錯。
    #[test]
    fn parse_stock_page_rejects_unrelated_page() {
        let error = parse_stock_price_html(
            "2884",
            "https://example.test/stock/sid2884.html",
            "<html><body><p>查無此股</p></body></html>",
        )
        .unwrap_err();
        assert!(error.to_string().contains("找不到股票"));
    }

    #[tokio::test]
    #[ignore = "live test：連線真實外部網站，需要時手動執行"]
    async fn test_get_stock_price() {
        dotenvy::dotenv().ok();
        log_stock_price_test::<PcHome>("2330").await;
    }

    #[tokio::test]
    #[ignore = "live test：連線真實外部網站，需要時手動執行"]
    async fn test_get_stock_quotes() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 megatime::get_stock_quotes");

        match PcHome::get_stock_quotes("2330").await {
            Ok(e) => {
                dbg!(&e);
                tracing::debug!("megatime::get_stock_quotes : {:#?}", e);
            }
            Err(why) => {
                dbg!(&why);
                tracing::debug!("Failed to megatime::get_stock_quotes because {:?}", why);
            }
        }

        tracing::debug!("結束 megatime::get_stock_quotes");
    }
}
