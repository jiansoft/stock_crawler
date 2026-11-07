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
            value
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
            value
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
///
/// 2026-11 改版後漲跌欄不再帶正負號（下跌也只寫「20」，方向改用 CSS class 表示），
/// 漲跌幅「(-0.88%)」仍帶號，所以漲跌的方向一律跟漲跌幅走；舊版帶號的頁面結果不變。
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
            change_range_raw
        ));
    } else {
        text::parse_f64(change_range_raw, Some(['(', ')'].to_vec()))?
    };
    let change = if change_range < 0.0 {
        -change.abs()
    } else {
        change.abs()
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
mod tests;
