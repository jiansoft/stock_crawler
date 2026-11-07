//! PCHome（megatime）價格爬蟲的測試；連外的測試標 #[ignore]。

use super::*;
use crate::infra::crawler::log_stock_price_test;
use rust_decimal_macros::dec;

/// 以貼近真實個股頁形狀的 fixture 驗證整頁解析流程（下跌情境）。
#[test]
fn parse_stock_page_fixture_extracts_price_and_quotes() {
    // include_str! 的路徑相對於本檔案（megatime/price/tests.rs）→ megatime/testdata/。
    const FIXTURE: &str = include_str!("../testdata/stock_page.html");
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

/// 錯誤訊息只帶單行的頁面標題，不可夾帶 HTML 或換行。
#[test]
fn missing_container_error_is_single_line_with_title() {
    let html = "<html><head>\n<meta charset=\"UTF-8\">\n<title>永記 (1726)\n 個股資訊 - PChome 股市</title>\n</head><body><p>x</p></body></html>";

    for error in [
        parse_stock_price_html("1726", "https://example.test", html).unwrap_err(),
        parse_stock_quotes_html("1726", "https://example.test", html).unwrap_err(),
    ] {
        let message = error.to_string();
        assert!(
            message.contains("永記 (1726) 個股資訊 - PChome 股市"),
            "{message}"
        );
        assert!(!message.contains('\n'), "{message}");
        assert!(!message.contains("<meta"), "{message}");
    }
}

/// 沒有標題時退回內文摘要，同樣是單行。
#[test]
fn page_summary_falls_back_to_body_text() {
    let document = Html::parse_document("<html><body><p>查無\n  此股</p></body></html>");
    assert_eq!(page_summary(&document), "（無標題）查無 此股");
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
