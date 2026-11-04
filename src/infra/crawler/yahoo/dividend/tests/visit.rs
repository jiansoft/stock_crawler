//! 以本機 mock server 取代 Yahoo 股利頁（G15），驗證 [`visit`] 的 404 分類與正常解析（C19）。

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::core::util::http::mock_origin::with_origins;

const ORIGIN: &str = "https://tw.stock.yahoo.com";

async fn visit_via(server: &MockServer, symbol: &str) -> Result<YahooDividend> {
    let uri = server.uri();
    with_origins(&[(ORIGIN, &uri)], visit(symbol)).await
}

/// 404 代表個股頁不存在，回傳可辨識的專屬錯誤，訊息帶代號與網址。
#[tokio::test]
async fn visit_returns_page_not_found_error_on_404() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote/00699R/dividend"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;

    let error = visit_via(&server, "00699R")
        .await
        .expect_err("404 應回錯誤");

    assert!(is_page_not_found_error(&error), "{error:#}");
    let message = error.to_string();
    assert!(
        message.contains("00699R") && message.contains("/quote/00699R/dividend"),
        "錯誤訊息要帶代號與網址，backfill 日誌才查得到是哪一檔：{message}"
    );
}

/// 200 時解析股利列，並可依發放年度取出明細。
#[tokio::test]
async fn visit_parses_dividend_rows_on_success() {
    let server = MockServer::start().await;
    let html = wrap_rows(&[dividend_row(
        "2024Q4",
        "1.5",
        "-",
        "2025/03/10",
        "-",
        "2025/04/11",
        "-",
    )]);
    Mock::given(method("GET"))
        .and(path("/quote/2330/dividend"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(html, "text/html; charset=utf-8"))
        .expect(1)
        .mount(&server)
        .await;

    let dividend = visit_via(&server, "2330")
        .await
        .expect("頁面存在應解析成功");

    assert_eq!(dividend.stock_symbol, "2330");
    let details = dividend
        .get_dividend_by_year(2025)
        .expect("除息日在 2025 年，應歸在 2025 發放年度");
    assert_eq!(details.len(), 1);
    assert_eq!(details[0].quarter, "Q4");
    assert_eq!(details[0].cash_dividend, dec!(1.5));
    assert_eq!(details[0].ex_dividend_date1, "2025-03-10");
    assert!(dividend.get_dividend_by_year(2024).is_none());
}

/// 頁面存在但找不到股利表（例如改版）時回一般錯誤，不能被當成 404 長期跳過。
#[tokio::test]
async fn visit_reports_parse_error_when_dividend_list_is_missing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote/2330/dividend"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw("<html><body>改版</body></html>", "text/html; charset=utf-8"),
        )
        .mount(&server)
        .await;

    let error = visit_via(&server, "2330")
        .await
        .expect_err("沒有股利表應回錯誤");

    assert!(!is_page_not_found_error(&error), "{error:#}");
}

/// `new` 建立的集合體沒有任何年度。
#[test]
fn new_starts_without_dividends() {
    let dividend = YahooDividend::new("2330".to_string());

    assert_eq!(dividend.stock_symbol, "2330");
    assert!(dividend.dividend.is_empty());
    assert!(dividend.get_dividend_by_year(2025).is_none());
}
