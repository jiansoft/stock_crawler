//! stock_weight的測試；連外的測試標 #[ignore]。

use super::*;
use rust_decimal_macros::dec;

/// 以貼近真實權重頁形狀的 fixture 驗證「一列兩檔」雙欄版面的解析。
#[test]
fn parse_stock_weight_html_parses_two_column_layout() {
    // include_str! 的路徑相對於本檔案（taifex/stock_weight/tests.rs）→ taifex/testdata/。
    const FIXTURE: &str = include_str!("../testdata/stock_weight.html");

    let result =
        parse_stock_weight_html(FIXTURE, "#printhere > div > div > table > tbody > tr").unwrap();

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

/// 期交所回非 2xx 時回錯誤（H38）：錯誤頁不能被當成「沒有權重」。
#[tokio::test]
async fn visit_rejects_non_success_status() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::core::util::http::mock_origin::with_origins;

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/cht/9/futuresQADetail"))
        .respond_with(ResponseTemplate::new(503).set_body_raw("<html>維護中</html>", "text/html"))
        .expect(1)
        .mount(&server)
        .await;

    let uri = server.uri();
    let error = with_origins(
        &[("https://www.taifex.com.tw", &uri)],
        visit(StockExchange::TWSE),
    )
    .await
    .expect_err("503 應回錯誤");

    assert!(error.to_string().contains("HTTP 503"), "{error:#}");
}
