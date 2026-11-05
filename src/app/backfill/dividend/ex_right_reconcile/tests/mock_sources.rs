//! 以本機 mock server 取代 TWSE、TPEx 與 Yahoo（G15）：除權息核對排程入口的完整流程。

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::core::util::http::mock_origin::with_origins;

const TWSE: &str = "https://www.twse.com.tw";
const TPEX: &str = "https://www.tpex.org.tw";
const YAHOO: &str = "https://tw.stock.yahoo.com";
/// 資料庫有、除息日延後的股票；交易所有、資料庫沒有的股票。
const POSTPONED: &str = "79979XR";
const MISSING: &str = "79979XM";

fn day(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(1991, month, day).expect("固定日期合法")
}

/// TWT49U：兩檔 1991 年（民國 80 年）的純除息結果。
fn twse_results() -> serde_json::Value {
    let event = |date: &str, symbol: &str, cash: &str| {
        serde_json::json!([
            date,
            symbol,
            "測試",
            "30.00",
            "28.50",
            cash,
            "息",
            "31.35",
            "25.65",
            "28.50",
            "28.50",
            format!("{symbol},19910804")
        ])
    };
    serde_json::json!({
        "stat": "OK",
        "data": [event("80年08月04日", POSTPONED, "1.503500"), event("80年09月10日", MISSING, "2.000000")]
    })
}

/// 三個來源：TWSE 兩筆、TPEx 沒有資料（`tpex_status` 非 200 時回錯誤）、Yahoo 一律 404。
async fn sources(tpex_status: u16) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rwd/zh/exRight/TWT49U"))
        .respond_with(ResponseTemplate::new(200).set_body_json(twse_results()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/www/zh-tw/bulletin/exDailyQ"))
        .respond_with(
            ResponseTemplate::new(tpex_status)
                .set_body_json(serde_json::json!({ "tables": [{ "data": [] }] })),
        )
        .mount(&server)
        .await;
    server
}

async fn reconcile(server: &MockServer, apply: bool) -> Result<ReconcileSummary> {
    let uri = server.uri();
    with_origins(
        &[(TWSE, &uri), (TPEX, &uri), (YAHOO, &uri)],
        execute(day(1, 1), day(12, 31), apply),
    )
    .await
}

/// 種入除息日還是舊日期（07-14）的 1991 年股利。
async fn seed_postponed() {
    let mut original = row(0, 1991, "", dec!(1.5), Decimal::ZERO, "1991-07-14", "-");
    original.security_code = POSTPONED.to_string();
    PgDividendRepository::new()
        .save(&original)
        .await
        .expect("種入股利");
}

async fn cleanup() {
    delete_test_dividends(POSTPONED).await;
    delete_test_dividends(MISSING).await;
}

/// 讀回延後那檔所有資料列的 (期別, 現金除息日, 現金股利)。
async fn postponed_rows() -> Vec<(String, String, Decimal)> {
    sqlx::query_as(
        r#"SELECT quarter, "ex-dividend_date1", cash_dividend FROM dividend
           WHERE security_code = $1 ORDER BY quarter"#,
    )
    .bind(POSTPONED)
    .fetch_all(crate::infra::database::get_connection())
    .await
    .expect("讀回測試股利")
}

async fn database_unavailable(test: &str) -> bool {
    dotenvy::dotenv().ok();
    if crate::infra::database::ping().await.is_err() {
        println!("跳過 {test}：無資料庫連接");
        return true;
    }
    false
}

/// 寫入模式：延後的除息日改成交易所日期；資料庫沒有、Yahoo 也補不到的事件列為未解決。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn execute_applies_official_dates_and_reports_unresolved() {
    if database_unavailable("execute_applies_official_dates_and_reports_unresolved").await {
        return;
    }
    cleanup().await;
    seed_postponed().await;
    let server = sources(200).await;

    let summary = reconcile(&server, true).await;
    let rows = postponed_rows().await;
    cleanup().await;

    let summary = summary.expect("核對完成");
    assert_eq!(
        (summary.official, summary.updated, summary.missing),
        (2, 1, 1),
        "{summary:?}"
    );
    assert_eq!((summary.filled, summary.unresolved), (0, 1), "{summary:?}");
    assert_eq!(
        rows,
        vec![("".to_owned(), "1991-08-04".to_owned(), dec!(1.5035))],
        "除息日與金額改成交易所結果"
    );
}

/// 只回報模式：同樣算出異動，但資料庫不動。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn execute_report_only_leaves_database_untouched() {
    if database_unavailable("execute_report_only_leaves_database_untouched").await {
        return;
    }
    cleanup().await;
    seed_postponed().await;
    let server = sources(200).await;

    let summary = reconcile(&server, false).await;
    let rows = postponed_rows().await;
    cleanup().await;

    let summary = summary.expect("核對完成");
    assert_eq!((summary.updated, summary.missing), (1, 1), "{summary:?}");
    assert_eq!(
        rows,
        vec![("".to_owned(), "1991-07-14".to_owned(), dec!(1.5))],
        "只回報時不改寫"
    );
}

/// 任一交易所來源失敗就回錯誤，不寫入任何資料。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn execute_fails_without_writing_when_a_source_fails() {
    if database_unavailable("execute_fails_without_writing_when_a_source_fails").await {
        return;
    }
    cleanup().await;
    seed_postponed().await;
    let server = sources(500).await;

    let result = reconcile(&server, true).await;
    let rows = postponed_rows().await;
    cleanup().await;

    assert!(result.is_err(), "TPEx 失敗要回錯誤");
    assert_eq!(rows[0].1, "1991-07-14", "來源失敗時不改寫");
}
