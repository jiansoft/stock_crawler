//! 以本機 mock server 取代 wespai 與 nstock（G15）：年度財報回補的略過旗標、空頁與寫入分支。

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::{
    core::util::http::mock_origin::with_origins,
    infra::{crawler::wespai::profit::Profit, database, nosql::redis::CLIENT},
};

const WESPAI: &str = "https://stock.wespai.com";
const NSTOCK: &str = "https://www.nstock.tw";
/// 固定的歷史年度，不使用今天的日期當資料日期。
const YEAR: i32 = 1990;
/// 新年報、資料庫已有的年報；代號不能有英文字母，否則會被當成特別股濾掉。
const NEW: &str = "7997901";
const EXISTING: &str = "7997902";
/// 特別股（代號含英文字母），不寫入。
const PREFERENCE: &str = "79979FA";
const SYMBOLS: [&str; 3] = [NEW, EXISTING, PREFERENCE];

/// wespai 年報頁：`rows` 依序為代號與 EPS，其餘欄位固定。
fn profit_page(rows: &[(&str, &str)]) -> String {
    let body: String = rows
        .iter()
        .map(|(symbol, eps)| {
            format!(
                "<tr><td>{symbol}</td><td>測試</td><td>10</td><td>30</td><td>20</td><td>15</td>\
                 <td>12</td><td>25</td><td>40</td><td>-</td><td>3</td><td>8.5</td><td>4.2</td>\
                 <td>{eps}</td></tr>"
            )
        })
        .collect();
    format!(
        "<html><body><h1><a href=\"/profit\">{YEAR}年報獲利能力</a></h1>\
         <table id=\"example\"><tbody>{body}</tbody></table></body></html>"
    )
}

/// 回應 wespai 年報頁的 mock server；nstock 沒有 mock，查詢一律 404。
async fn wespai_server(status: u16, page: String) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/profit"))
        .respond_with(ResponseTemplate::new(status).set_body_string(page))
        .mount(&server)
        .await;
    server
}

async fn run_with(server: &MockServer) -> Result<()> {
    let uri = server.uri();
    with_origins(&[(WESPAI, &uri), (NSTOCK, &uri)], run()).await
}

async fn cleanup() {
    sqlx::query("DELETE FROM financial_statement WHERE security_code = ANY($1) AND year = $2")
        .bind(SYMBOLS)
        .bind(YEAR)
        .execute(database::get_connection())
        .await
        .expect("清除測試年報");
    let _ = CLIENT.delete(ANNUAL_SKIP_KEY).await;
}

/// 讀回測試年度的年報代號與 EPS，依代號排序。
async fn saved() -> Vec<(String, Decimal)> {
    sqlx::query_as(
        "SELECT security_code, earnings_per_share FROM financial_statement
         WHERE security_code = ANY($1) AND year = $2 AND quarter = '' ORDER BY security_code",
    )
    .bind(SYMBOLS)
    .bind(YEAR)
    .fetch_all(database::get_connection())
    .await
    .expect("讀回測試年報")
}

async fn services_unavailable(test: &str) -> bool {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() || CLIENT.ping().await.is_err() {
        println!("跳過 {test}：無資料庫或 Redis 連接");
        return true;
    }
    false
}

/// 只寫入資料庫還沒有的普通股年報，已有的不覆寫、特別股略過；完成後設一週的略過旗標。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn run_saves_only_new_common_stock_statements() {
    if services_unavailable("run_saves_only_new_common_stock_statements").await {
        return;
    }
    cleanup().await;
    let mut existing = Profit::new(YEAR, EXISTING.to_owned());
    existing.earnings_per_share = dec!(1.11);
    existing.return_on_equity = dec!(5);
    existing.return_on_assets = dec!(3);
    PgFinancialRepository::new()
        .batch_save_financial_statements(&[FinancialStatementAclMapper::from_wespai(existing)])
        .await
        .expect("種入既有年報");
    let server = wespai_server(
        200,
        profit_page(&[(NEW, "2.50"), (EXISTING, "9.99"), (PREFERENCE, "3.00")]),
    )
    .await;

    let result = run_with(&server).await;
    let rows = saved().await;
    let flagged = CLIENT.get_bool(ANNUAL_SKIP_KEY).await;
    cleanup().await;

    result.expect("年度財報回補完成");
    assert_eq!(
        rows,
        vec![
            (NEW.to_owned(), dec!(2.50)),
            (EXISTING.to_owned(), dec!(1.11)),
        ],
        "新年報寫入、既有年報不覆寫、特別股不寫"
    );
    assert!(flagged.expect("讀取略過旗標"), "完成後要設略過旗標");
}

/// 略過旗標還在時不抓 wespai。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn run_skips_while_flag_is_set() {
    if services_unavailable("run_skips_while_flag_is_set").await {
        return;
    }
    cleanup().await;
    CLIENT
        .set(ANNUAL_SKIP_KEY, true, 60)
        .await
        .expect("設定略過旗標");
    let server = wespai_server(200, profit_page(&[(NEW, "2.50")])).await;

    let result = run_with(&server).await;
    cleanup().await;

    result.expect("略過不算錯誤");
    let requests = server.received_requests().await.unwrap_or_default();
    assert!(requests.is_empty(), "旗標還在時不抓 wespai");
}

/// wespai 頁面沒有任何列時直接結束、不設旗標；頁面改版（找不到年度）回錯誤。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn run_stops_on_empty_or_changed_page() {
    if services_unavailable("run_stops_on_empty_or_changed_page").await {
        return;
    }
    cleanup().await;

    let empty = wespai_server(200, profit_page(&[])).await;
    run_with(&empty).await.expect("沒有資料列不算錯誤");
    let flagged = CLIENT
        .get_bool(ANNUAL_SKIP_KEY)
        .await
        .expect("讀取略過旗標");
    assert!(!flagged, "沒有資料時不設旗標，下一輪再抓");

    let changed = wespai_server(500, "<html><body>維護中</body></html>".to_owned()).await;
    let err = run_with(&changed).await.expect_err("找不到年度要回錯誤");
    cleanup().await;
    assert!(err.to_string().contains("select"), "{err:#}");
}

/// 排程入口：平日交給 `run()`（旗標還在，所以不抓 wespai）；週末直接結束。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn execute_defers_to_run_on_weekdays() {
    if services_unavailable("execute_defers_to_run_on_weekdays").await {
        return;
    }
    CLIENT
        .set(ANNUAL_SKIP_KEY, true, 60)
        .await
        .expect("設定略過旗標");
    let server = wespai_server(200, profit_page(&[(NEW, "2.50")])).await;
    let uri = server.uri();

    let result = with_origins(&[(WESPAI, &uri), (NSTOCK, &uri)], execute()).await;
    cleanup().await;

    result.expect("排程入口不回錯誤");
    let requests = server.received_requests().await.unwrap_or_default();
    assert!(requests.is_empty(), "旗標還在時不抓 wespai");
}
