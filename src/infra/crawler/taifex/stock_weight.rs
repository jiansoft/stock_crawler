use core::result::Result::Ok;

use anyhow::{Result, anyhow};
use reqwest::header::HeaderMap;
use rust_decimal::Decimal;
use scraper::{ElementRef, Html, Selector};

use crate::{
    core::declare::StockExchange,
    core::util::{self, http::element},
    infra::crawler::{taifex, taifex::HOST},
};

#[derive(Default, Debug, Clone, PartialEq)]
/// 臺指期貨網站揭露的單檔權重資料。
pub struct StockWeight {
    /// 權重排名。
    pub rank: i32,
    /// 股票代號。
    pub stock_symbol: String,
    /// 權重百分比。
    pub weight: Decimal,
}

struct ExchangeConfig {
    url: String,
    selector: String,
}

impl ExchangeConfig {
    /// 建立一個新的交易所組態實例。
    ///
    /// # 參數
    ///
    /// * `exchange`: 一個 `StockExchange` 列舉，代表選擇的股票交易所。
    ///
    /// 返回: 返回一個新的 `ExchangeConfig` 實例，其中包含了訪問特定交易所資料所需的URL和選擇器。
    ///
    /// # 示例
    ///
    /// ```ignore
    /// let config = ExchangeConfig::new(StockExchange::TWSE);
    /// println!("URL: {}", config.url);
    /// println!("Selector: {}", config.selector);
    /// ```
    ///
    /// 上面的程式碼展示了如何建立一個針對台灣證券交易所 (TWSE) 的組態實例，並列印相關的 URL 和選擇器。
    fn new(exchange: StockExchange) -> Self {
        match exchange {
            StockExchange::TWSE => Self {
                url: format!("https://{}/cht/9/futuresQADetail", taifex::HOST),
                selector: "#printhere > div > div > table > tbody > tr".to_string(),
            },
            StockExchange::TPEx => Self {
                url: format!("https://{}/cht/2/tPEXPropertion", taifex::HOST),
                selector: "#printhere > div > table > tbody > tr".to_string(),
            },
            _ => panic!("Unsupported exchange"),
        }
    }
}

/// 台股各股權重
///
/// 只負責 HTTP 抓取，解析交給 [`parse_stock_weight_html`]
/// （fetch/parse 分離，解析邏輯才能被 fixture 單元測試覆蓋）。
pub async fn visit(exchange: StockExchange) -> Result<Vec<StockWeight>> {
    let exchange_market = ExchangeConfig::new(exchange);
    let url = &exchange_market.url;
    let ua = util::http::user_agent::gen_random_ua();
    let mut headers = HeaderMap::new();

    headers.insert("Host", HOST.parse()?);
    headers.insert("Referer", url.parse()?);
    headers.insert("User-Agent", ua.parse()?);

    let text = util::http::get(url, Some(headers)).await?;

    if text.is_empty() {
        return Ok(Vec::new());
    }

    parse_stock_weight_html(&text, &exchange_market.selector)
}

/// 解析臺指期網站權重頁的 HTML。
///
/// 這是一個「純函式」——輸入只有 HTML 字串與資料列 selector
/// （上市與上櫃頁的表格層級不同，由 [`ExchangeConfig`] 提供），
/// 不做任何網路 I/O，可用 `testdata/stock_weight.html` fixture 直接驗證。
///
/// # 解析規則
/// 期交所把權重表排成「一列兩檔」的雙欄版面：
/// - 左半：td 1＝排名、td 2＝代號、td 3＝名稱、td 4＝權重。
/// - 右半：td 5＝排名、td 6＝代號、td 7＝名稱、td 8＝權重。
///
/// 每列因此最多解析出兩筆 [`StockWeight`]；代號空白或權重為 0
/// （例如最後一列右半為空）的半邊由 [`get_stock_weight`] 回傳 `None` 略過。
fn parse_stock_weight_html(text: &str, row_selector: &str) -> Result<Vec<StockWeight>> {
    let mut result: Vec<StockWeight> = Vec::with_capacity(1024);
    let document = Html::parse_document(text);
    let selector = match Selector::parse(row_selector) {
        Ok(selector) => selector,
        Err(why) => {
            return Err(anyhow!("Failed to Selector::parse because: {:?}", why));
        }
    };

    document.select(&selector).for_each(|element| {
        if let Some(sw) = get_stock_weight(
            &element,
            "td:nth-child(1)",
            "td:nth-child(2)",
            "td:nth-child(4)",
        ) {
            result.push(sw);
        }
        if let Some(sw) = get_stock_weight(
            &element,
            "td:nth-child(5)",
            "td:nth-child(6)",
            "td:nth-child(8)",
        ) {
            result.push(sw);
        }
    });

    Ok(result)
}

/// Parses stock weight from an HTML element.
///
/// # Arguments
/// * `element` - A reference to the element to parse from.
/// * `rank_selector` - CSS selector to find the rank.
/// * `symbol_selector` - CSS selector to find the stock symbol.
/// * `weight_selector` - CSS selector to find the weight.
///
/// # Returns `Some(StockWeight)` if parsing succeeds, otherwise `None`.
fn get_stock_weight(
    element: &ElementRef,
    rank_selector: &str,
    symbol_selector: &str,
    weight_selector: &str,
) -> Option<StockWeight> {
    let stock_symbol = element::parse_to_string(element, symbol_selector);
    let weight = element::parse_to_decimal(element, weight_selector);

    if !stock_symbol.is_empty() && !weight.is_zero() {
        let sw = StockWeight {
            rank: element::parse_to_i32(element, rank_selector),
            stock_symbol,
            weight,
        };

        return Some(sw);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    /// 以貼近真實權重頁形狀的 fixture 驗證「一列兩檔」雙欄版面的解析。
    #[test]
    fn parse_stock_weight_html_parses_two_column_layout() {
        // include_str! 的路徑相對於本檔案（taifex/stock_weight.rs）→ taifex/testdata/。
        const FIXTURE: &str = include_str!("testdata/stock_weight.html");

        let result =
            parse_stock_weight_html(FIXTURE, "#printhere > div > div > table > tbody > tr")
                .unwrap();

        // 表頭列（權重解析為 0）與最後一列的空白右半都該被略過，
        // 有效資料為 3 檔：一列兩檔 × 1 + 奇數尾列左半 × 1。
        assert_eq!(result.len(), 3);

        assert_eq!(result[0].rank, 1);
        assert_eq!(result[0].stock_symbol, "2330");
        assert_eq!(result[0].weight, dec!(34.61)); // 「34.61%」的 % 由預設清單去除

        assert_eq!(result[1].rank, 2);
        assert_eq!(result[1].stock_symbol, "2317");

        assert_eq!(result[2].rank, 3);
        assert_eq!(result[2].stock_symbol, "2454");
        assert_eq!(result[2].weight, dec!(3.12));
    }

    /// 與目標結構無關的頁面應回傳空清單，不 panic。
    #[test]
    fn parse_stock_weight_html_returns_empty_for_unrelated_page() {
        let result = parse_stock_weight_html(
            "<html><body><p>系統維護中</p></body></html>",
            "#printhere > div > div > table > tbody > tr",
        )
        .unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 visit");

        match visit(StockExchange::TWSE).await {
            Ok(e) => {
                dbg!(&e);
                tracing::debug!("len:{}\r\n {:#?}", e.len(), e);
            }
            Err(why) => {
                tracing::debug!("Failed to visit because {:?}", why);
            }
        }

        tracing::debug!("結束 visit");
    }
}
