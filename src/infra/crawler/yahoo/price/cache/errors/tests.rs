//! Yahoo 類股 API 的錯誤分類：WAF 阻擋（需要整輪冷卻與警報）與 5xx 暫時性錯誤（只記 warn）的測試。

use super::*;

#[test]
fn is_transient_server_error_matches_only_5xx() {
    assert!(is_transient_server_error(
        "Yahoo 類股 API request failed with status 500 Internal Server Error for https://x. Body: "
    ));
    assert!(is_transient_server_error(
        "Yahoo 類股 API request failed with status 502 Bad Gateway for https://x. Body: "
    ));
    assert!(!is_transient_server_error(
        "Yahoo 類股 API request failed with status 404 Not Found for https://x. Body: "
    ));
    assert!(!is_transient_server_error("Request denied"));
}

/// 遭遇阻擋後寫入 1 小時的警報旗標；旗標還在時再次呼叫不會重設也不會出錯。
#[tokio::test]
async fn alert_denied_once_sets_the_alert_flag() {
    let category = YahooClassCategory::enabled(
        crate::infra::crawler::yahoo::YahooClassExchange::Listed,
        40,
        "半導體",
    );

    alert_denied_once(&category, "Request denied");
    assert!(TTL.daily_quote_contains_key("alert:yahoo:denied"));
    alert_denied_once(&category, "Request denied");
    assert!(TTL.daily_quote_contains_key("alert:yahoo:denied"));
}

#[test]
fn is_denied_matches_waf_responses_only() {
    assert!(is_denied("Yahoo 類股 API Request denied by WAF"));
    assert!(is_denied(
        "Yahoo 類股 API request failed with status 999 Unknown for https://x. Body: "
    ));
    assert!(!is_denied(
        "Yahoo 類股 API request failed with status 500 Internal Server Error for https://x. Body: "
    ));
}
