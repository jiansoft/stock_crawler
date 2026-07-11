use anyhow::{Result, anyhow};
use async_trait::async_trait;
use reqwest::header::{self, HeaderValue};
use rust_decimal::Decimal;
use scraper::Html;

use crate::{
    core::declare,
    core::util::{self, text},
    infra::crawler::{
        StockInfo,
        cmoney::{CMoney, HOST},
    },
};

/// 建立 CMoney 個股頁面的請求標頭。
///
/// 透過補齊常見瀏覽器標頭（例如 `Accept`、`Accept-Language`、
/// `Referer`），降低請求在連線層或防爬機制被拒絕的機率。
fn build_stock_page_headers() -> header::HeaderMap {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::ACCEPT,
        HeaderValue::from_static(
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
        ),
    );
    headers.insert(
        header::ACCEPT_LANGUAGE,
        HeaderValue::from_static("zh-TW,zh;q=0.9,en-US;q=0.8,en;q=0.7"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(
        header::REFERER,
        HeaderValue::from_static("https://www.cmoney.tw/forum/stock"),
    );
    headers.insert(
        header::UPGRADE_INSECURE_REQUESTS,
        HeaderValue::from_static("1"),
    );
    headers
}

/// CMoney 即時報價抓取實作。
///
/// 此實作會抓取 CMoney 個股頁面，解析當前股價與漲跌資訊。
fn parse_required_decimal(raw: &str, stock_symbol: &str, field_name: &str) -> Result<Decimal> {
    let value = raw.trim();
    if value.is_empty() || value == "-" {
        return Err(anyhow!(
            "CMoney field `{}` is unavailable for stock {}: {:?}",
            field_name,
            stock_symbol,
            raw
        ));
    }

    text::parse_decimal(value, None)
}

fn parse_required_f64(raw: &str, stock_symbol: &str, field_name: &str) -> Result<f64> {
    let value = raw.trim();
    if value.is_empty() || value == "-" {
        return Err(anyhow!(
            "CMoney field `{}` is unavailable for stock {}: {:?}",
            field_name,
            stock_symbol,
            raw
        ));
    }

    text::parse_f64(value, None)
}

/// 解析 CMoney 個股頁 HTML 中的即時價格。
///
/// 這是一個「純函式」——輸入只有 HTML 字串，不做任何網路 I/O，
/// 可用 `testdata/stock_page.html` fixture 直接驗證（fetch/parse 分離）。
fn parse_stock_price_html(stock_symbol: &str, url: &str, html: &str) -> Result<Decimal> {
    let document = Html::parse_document(html);
    let target = util::http::element::GetOneElementText {
        stock_symbol,
        url,
        selector: "section > div",
        element: "div.stockData__info > div",
        document,
    };

    let price = util::http::element::get_one_element(target)?;
    parse_required_decimal(&price, stock_symbol, "price")
}

/// 解析 CMoney 個股頁 HTML 中的完整報價（價格、漲跌、漲跌幅）。
///
/// 與 [`parse_stock_price_html`] 一樣是純函式。三個欄位分開取值，
/// 讓錯誤訊息能明確指出是哪一個欄位的 selector 失效。
fn parse_stock_quotes_html(
    stock_symbol: &str,
    url: &str,
    html: &str,
) -> Result<declare::StockQuotes> {
    let document = Html::parse_document(html);

    let price = util::http::element::get_one_element(util::http::element::GetOneElementText {
        stock_symbol,
        url,
        selector: "section > div",
        element: "div.stockData__info > div",
        document: document.clone(),
    })?;
    let price = parse_required_f64(&price, stock_symbol, "price")?;

    let change = util::http::element::get_one_element(util::http::element::GetOneElementText {
        stock_symbol,
        url,
        selector: r"section > div",
        element: r"div.stockData__info > div.stockData__value > div.stockData__quotePrice",
        document: document.clone(),
    })?;
    let change = parse_required_f64(&change, stock_symbol, "change")?;

    let change_range =
        util::http::element::get_one_element(util::http::element::GetOneElementText {
            stock_symbol,
            url,
            selector: r"section > div",
            element: r"div.stockData__info > div.stockData__value > div.stockData__quote",
            document,
        })?;
    let change_range_raw = change_range.trim();
    let change_range = if change_range_raw.is_empty() || change_range_raw == "-" {
        return Err(anyhow!(
            "CMoney field `change_range` is unavailable for stock {}: {:?}",
            stock_symbol,
            change_range
        ));
    } else {
        text::parse_f64(change_range_raw, Some(['(', ')'].to_vec()))?
    };

    Ok(declare::StockQuotes {
        stock_symbol: stock_symbol.to_string(),
        price,
        change,
        change_range,
    })
}

#[async_trait]
impl StockInfo for CMoney {
    /// 取得單一股票的即時價格。
    ///
    /// 只負責 HTTP 抓取，解析交給 [`parse_stock_price_html`]。
    /// 會回傳解析後的十進位價格；若網頁結構或內容異常則回傳錯誤。
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        let url = format!(
            "https://{host}/forum/stock/{symbol}",
            host = HOST,
            symbol = stock_symbol
        );
        let text = util::http::get(&url, Some(build_stock_page_headers())).await?;
        parse_stock_price_html(stock_symbol, &url, &text)
    }

    /// 取得單一股票的即時報價資訊。
    ///
    /// 只負責 HTTP 抓取，解析交給 [`parse_stock_quotes_html`]。
    /// 包含目前價格、漲跌價差與漲跌幅百分比。
    async fn get_stock_quotes(stock_symbol: &str) -> Result<declare::StockQuotes> {
        let url = &format!(
            "https://{host}/forum/stock/{symbol}",
            host = HOST,
            symbol = stock_symbol
        );
        let text = util::http::get(url, Some(build_stock_page_headers())).await?;
        parse_stock_quotes_html(stock_symbol, url, &text)
    }
}

#[cfg(test)]
/// CMoney 報價抓取相關測試。
///
/// 這些測試需連線外部網站，執行結果會受網路與來源頁面變動影響。
mod tests {
    use super::*;
    use crate::infra::crawler::log_stock_price_test;

    /// 以貼近真實個股頁形狀的 fixture 驗證整頁解析流程（下跌情境）。
    #[test]
    fn parse_stock_page_fixture_extracts_price_and_quotes() {
        // include_str! 的路徑相對於本檔案（cmoney/price.rs）→ cmoney/testdata/。
        const FIXTURE: &str = include_str!("testdata/stock_page.html");
        let url = "https://www.cmoney.tw/forum/stock/2884";

        let price = parse_stock_price_html("2884", url, FIXTURE).unwrap();
        assert_eq!(price, rust_decimal_macros::dec!(173.5));

        let quotes = parse_stock_quotes_html("2884", url, FIXTURE).unwrap();
        assert_eq!(quotes.stock_symbol, "2884");
        assert_eq!(quotes.price, 173.5);
        assert_eq!(quotes.change, -2.5);
        // 漲跌幅原文是「(-1.42%)」，括號與 % 都要被去除。
        assert_eq!(quotes.change_range, -1.42);
    }

    /// 頁面缺少報價容器（改版或載到錯誤頁）時必須明確報錯。
    #[test]
    fn parse_stock_page_rejects_unrelated_page() {
        let error = parse_stock_price_html(
            "2884",
            "https://example.test/forum/stock/2884",
            "<html><body><p>404</p></body></html>",
        )
        .unwrap_err();
        assert!(error.to_string().contains("element not found"));
    }

    #[test]
    fn test_parse_required_decimal_rejects_dash() {
        let err = parse_required_decimal("-", "5306", "price")
            .expect_err("dash should be treated as unavailable");
        assert!(err.to_string().contains("field `price` is unavailable"));
        assert!(err.to_string().contains("5306"));
    }

    #[test]
    fn test_parse_required_f64_rejects_dash() {
        let err =
            parse_required_f64("-", "5306", "change").expect_err("dash should be unavailable");
        assert!(err.to_string().contains("field `change` is unavailable"));
        assert!(err.to_string().contains("5306"));
    }

    #[tokio::test]
    #[ignore = "live test：連線真實外部網站，需要時手動執行"]
    /// 測試可取得指定股票即時價格。
    async fn test_get_stock_price() {
        dotenvy::dotenv().ok();
        log_stock_price_test::<CMoney>("4438").await;
    }

    #[tokio::test]
    #[ignore = "live test：連線真實外部網站，需要時手動執行"]
    /// 測試可取得指定股票完整即時報價。
    async fn test_get_stock_quotes() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 cmoney::get_stock_quotes");

        match CMoney::get_stock_quotes("4438").await {
            Ok(e) => {
                dbg!(&e);
                tracing::debug!("cmoney::get_stock_quotes : {:#?}", e);
            }
            Err(why) => {
                dbg!(&why);
                tracing::debug!("Failed to cmoney::get_stock_quotes because {:?}", why);
            }
        }

        tracing::debug!("結束 cmoney::get_stock_quotes");
    }
}
