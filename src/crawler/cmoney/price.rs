use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::header::{self, HeaderValue};
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
        let text = util::http::get(&url, Some(build_stock_page_headers())).await?;
        let document = Html::parse_document(&text);
        let target = util::http::element::GetOneElementText {
            stock_symbol,
            url: &url,
            selector: "section > div",
            element: "div.stockData__info > div",
            document,
        };

        let price = util::http::element::get_one_element(target)?;
        parse_required_decimal(&price, stock_symbol, "price")
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
        let text = util::http::get(url, Some(build_stock_page_headers())).await?;
        let document = Html::parse_document(&text);

        let price = util::http::element::get_one_element(util::http::element::GetOneElementText {
            stock_symbol,
            url,
            selector: "section > div",
            element: "div.stockData__info > div",
            document: document.clone(),
        })?;
        let price = parse_required_f64(&price, stock_symbol, "price")?;

        let change =
            util::http::element::get_one_element(util::http::element::GetOneElementText {
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
                document: document.clone(),
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
}

#[cfg(test)]
/// CMoney 報價抓取相關測試。
///
/// 這些測試需連線外部網站，執行結果會受網路與來源頁面變動影響。
mod tests {
    use super::*;
    use crate::{crawler::log_stock_price_test, logging};

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
    /// 測試可取得指定股票即時價格。
    async fn test_get_stock_price() {
        dotenv::dotenv().ok();
        log_stock_price_test::<CMoney>("4438").await;
    }

    #[tokio::test]
    /// 測試可取得指定股票完整即時報價。
    async fn test_get_stock_quotes() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 cmoney::get_stock_quotes".to_string());

        match CMoney::get_stock_quotes("4438").await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("cmoney::get_stock_quotes : {:#?}", e));
            }
            Err(why) => {
                dbg!(&why);
                logging::debug_file_async(format!(
                    "Failed to cmoney::get_stock_quotes because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 cmoney::get_stock_quotes".to_string());
    }
}
