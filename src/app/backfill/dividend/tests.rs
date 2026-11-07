//! 股利回補入口的測試：ETN 判斷、除權息公告掃描與交易所核對的排程入口。

use super::*;

#[test]
fn test_is_exchange_traded_note() {
    assert!(is_exchange_traded_note("020035"));
    assert!(is_exchange_traded_note("020001"));
    assert!(!is_exchange_traded_note("0050"));
    assert!(!is_exchange_traded_note("00400A"));
    assert!(!is_exchange_traded_note("2890"));
    assert!(!is_exchange_traded_note("02003B"));
}

/// 除權息公告掃描入口：上市、上櫃預告表都抓不到時回傳錯誤，不碰資料庫。
#[tokio::test]
async fn scan_announcements_fails_without_announcement_tables() {
    use crate::core::util::http::mock_origin::with_origins;

    let server = wiremock::MockServer::start().await;
    let uri = server.uri();

    let result = with_origins(
        &[
            ("https://openapi.twse.com.tw", &uri),
            ("https://www.tpex.org.tw", &uri),
            ("https://www.moneydj.com", &uri),
        ],
        scan_announcements(),
    )
    .await;

    assert!(result.is_err(), "兩個預告表都是 404");
}

/// 交易所核對入口：回看近 45 天，交易所兩邊都沒有除權息結果時完成且不寫入。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn reconcile_ex_right_results_completes_with_empty_exchange_results() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::core::util::http::mock_origin::with_origins;

    dotenvy::dotenv().ok();
    if crate::infra::database::ping().await.is_err() {
        println!(
            "跳過 reconcile_ex_right_results_completes_with_empty_exchange_results：無資料庫連接"
        );
        return;
    }
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rwd/zh/exRight/TWT49U"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "stat": "OK", "data": [] })),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/www/zh-tw/bulletin/exDailyQ"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "tables": [{ "data": [] }] })),
        )
        .mount(&server)
        .await;
    let uri = server.uri();

    let result = with_origins(
        &[
            ("https://www.twse.com.tw", &uri),
            ("https://www.tpex.org.tw", &uri),
            ("https://tw.stock.yahoo.com", &uri),
        ],
        reconcile_ex_right_results(),
    )
    .await;

    assert!(result.is_ok(), "{result:?}");
}
