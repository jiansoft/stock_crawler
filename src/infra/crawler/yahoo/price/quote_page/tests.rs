//! Yahoo 個股 Quote 頁解析的測試；連外的測試標 #[ignore]。

use super::*;
use rust_decimal_macros::dec;

/// 以貼近真實 Yahoo quote 頁形狀的 fixture 驗證下跌情境的解析。
///
/// 核心驗證點：Yahoo 頁面上的漲跌數字是「正數文字」（15、1.03%），
/// 實際方向靠顏色 class `C($c-trend-down)` 表現——解析器必須
/// 據此把漲跌與漲跌幅校正為負值，且千分位逗號要被正確去除。
#[test]
fn parse_quote_page_fixture_corrects_sign_by_trend_class() {
    // include_str! 的路徑相對於本檔案（yahoo/price/quote_page/tests.rs）
    // → 上一層的 yahoo/testdata/。
    const FIXTURE: &str = include_str!("../../testdata/quote_page.html");
    let url = "https://tw.stock.yahoo.com/quote/2330";

    // 大字成交價：'1,435' → 1435（去逗號、normalize）。
    let price = parse_quote_page_price("2330", url, FIXTURE).unwrap();
    assert_eq!(price, dec!(1435));

    // 完整報價：數字為正、顏色為跌 → 漲跌與漲跌幅都應轉為負值。
    let quotes = parse_quote_page_quotes("2330", url, FIXTURE).unwrap();
    assert_eq!(quotes.stock_symbol, "2330");
    assert_eq!(quotes.price, 1435.0);
    assert_eq!(quotes.change, -15.0);
    assert_eq!(quotes.change_range, -1.03);
}

/// 上漲情境：沒有下跌顏色 class 時，正數必須保持正數。
#[test]
fn parse_quote_page_keeps_positive_change_without_down_class() {
    let html = r#"
        <div id="main-0-QuoteHeader-Proxy">
            <span class="Fz(32px) C($c-trend-up)">990</span>
            <span class="Fz(20px) C($c-trend-up)">12.5</span>
            <span class="Jc(fe) C($c-trend-up)">(1.28%)</span>
        </div>
    "#;

    let quotes = parse_quote_page_quotes("2330", "https://example.test/quote/2330", html).unwrap();
    assert_eq!(quotes.change, 12.5);
    assert_eq!(quotes.change_range, 1.28);
}

/// 頁面缺少報價標頭（改版或載到錯誤頁）時必須明確報錯，
/// 而不是回傳 0 被誤當有效價格。
#[test]
fn parse_quote_page_rejects_page_without_quote_header() {
    let error = parse_quote_page_price(
        "2330",
        "https://example.test/quote/2330",
        "<html><body><p>頁面不存在</p></body></html>",
    )
    .unwrap_err();
    assert!(error.to_string().contains("element not found"));
}

/// Live 測試：驗證單檔 Yahoo quote 頁仍可抓到指定股票的成交價。
#[tokio::test]
#[ignore]
async fn test_get_stock_price() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 visit");

    match Yahoo::get_stock_price("2330").await {
        Ok(e) => {
            dbg!(&e);
            tracing::debug!("yahoo::get_stock_price {:#?}", e);
        }
        Err(why) => {
            tracing::debug!("Failed to visit because {:?}", why);
        }
    }

    tracing::debug!("結束 visit");
}

/// Live 測試：驗證單檔 Yahoo quote 頁仍可抓到成交價、漲跌與漲幅資訊。
#[tokio::test]
#[ignore]
async fn test_get_stock_quotes() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 yahoo::get_stock_quotes");

    match Yahoo::get_stock_quotes("2330").await {
        Ok(e) => {
            dbg!(&e);
            tracing::debug!("yahoo::get_stock_quotes {:#?}", e);
        }
        Err(why) => {
            tracing::debug!("Failed to yahoo::get_stock_quotes because {:?}", why);
        }
    }

    tracing::debug!("結束 yahoo::get_stock_quotes");
}
