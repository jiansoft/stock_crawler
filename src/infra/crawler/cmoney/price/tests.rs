//! CMoney 報價抓取的測試；連外的測試標 #[ignore]（結果受網路與來源頁面變動影響）。

use super::*;
use crate::infra::crawler::log_stock_price_test;

/// 以貼近真實個股頁形狀的 fixture 驗證整頁解析流程（下跌情境）。
#[test]
fn parse_stock_page_fixture_extracts_price_and_quotes() {
    // include_str! 的路徑相對於本檔案（cmoney/price/tests.rs）→ cmoney/testdata/。
    const FIXTURE: &str = include_str!("../testdata/stock_page.html");
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

/// 2026-11 改版後的頁面：成交價包在 span 裡、漲跌不帶正負號，下跌要依漲跌幅轉成負數。
#[test]
fn parse_current_stock_page_takes_change_sign_from_change_range() {
    const FIXTURE: &str = include_str!("../testdata/stock_page_2026_11.html");
    let url = "https://www.cmoney.tw/forum/stock/2330";

    let price = parse_stock_price_html("2330", url, FIXTURE).expect("成交價應解析成功");
    assert_eq!(price, rust_decimal_macros::dec!(2240));

    let quotes = parse_stock_quotes_html("2330", url, FIXTURE).expect("報價應解析成功");
    assert_eq!(
        (quotes.price, quotes.change, quotes.change_range),
        (2240.0, -20.0, -0.88),
        "頁面寫「20」「(-0.88%)」，漲跌應依漲跌幅轉成負數"
    );

    let rising = FIXTURE.replace("(-0.88%)", "(0.88%)");
    let quotes = parse_stock_quotes_html("2330", url, &rising).expect("上漲報價應解析成功");
    assert_eq!(
        (quotes.change, quotes.change_range),
        (20.0, 0.88),
        "上漲時漲跌維持正數"
    );
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
    let err = parse_required_f64("-", "5306", "change").expect_err("dash should be unavailable");
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
