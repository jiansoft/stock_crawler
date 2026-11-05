//! 以本機 mock server 取代 Yahoo 與 nstock（G15）：季報補值排程以固定歷史日期跑完整流程。

use chrono::{NaiveDate, TimeZone};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::core::util::http::mock_origin::with_origins;

const YAHOO: &str = "https://tw.stock.yahoo.com";
const NSTOCK: &str = "https://www.nstock.tw";
/// 1991-06-20：Q1 已過申報期限、還沒進入 Q2 預抓視窗，目標只有 1991-Q1。
const NOW: (i32, u32, u32) = (1991, 6, 20);
const SYMBOL: &str = "79979RF";
/// Yahoo 個股 profile 頁（2025 Q3 版面），換成測試季度後使用。
const PROFILE_PAGE: &str =
    include_str!("../../../../../infra/crawler/yahoo/testdata/profile_page.html");

fn now() -> DateTime<Local> {
    let (year, month, day) = NOW;
    let naive = NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|date| date.and_hms_opt(12, 0, 0))
        .expect("有效日期");
    Local
        .from_local_datetime(&naive)
        .single()
        .expect("本地時間沒有歧義")
}

/// 1991-Q1 ROE、ROA 都是 0 的財報。
fn zero_1991_q1() -> FinancialStatement {
    FinancialStatement {
        year: 1991,
        ..zero_statement(SYMBOL)
    }
}

async fn cleanup_1991(estimate_date: Option<&str>) {
    let pool = database::get_connection();
    for sql in [
        "DELETE FROM financial_statement WHERE security_code = $1",
        "DELETE FROM stocks WHERE stock_symbol = $1",
        "DELETE FROM estimate WHERE security_code = $1",
    ] {
        sqlx::query(sql)
            .bind(SYMBOL)
            .execute(pool)
            .await
            .unwrap_or_else(|why| panic!("{sql}: {why}"));
    }
    let _ = CLIENT
        .delete(&format!("FinancialStatement:{SYMBOL}-1991-Q1"))
        .await;
    let _ = CLIENT
        .delete(&yahoo::profile::no_valid_data_cache_key(SYMBOL))
        .await;
    match estimate_date {
        Some(val) => sqlx::query("UPDATE config SET val = $2 WHERE key = $1").bind(val),
        None => sqlx::query("DELETE FROM config WHERE key = $1"),
    }
    .bind("estimate-date")
    .execute(pool)
    .await
    .expect("還原估價日期設定");
}

/// 目標季度 ROE／ROA 為 0 的列由 Yahoo profile 補上，接著重算 EPS／ROE 與估價。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn execute_at_fills_target_quarter_and_recalculates_estimates() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() || CLIENT.ping().await.is_err() {
        println!(
            "跳過 execute_at_fills_target_quarter_and_recalculates_estimates：無資料庫或 Redis 連接"
        );
        return;
    }
    let pool = database::get_connection();
    let estimate_date: Option<String> =
        sqlx::query_scalar("SELECT val FROM config WHERE key = 'estimate-date'")
            .fetch_optional(pool)
            .await
            .expect("讀取估價日期設定");
    cleanup_1991(estimate_date.as_deref()).await;
    sqlx::query(
        r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, "SuspendListing") VALUES ($1, '季報排程測試', $1, 2, false)"#,
    )
    .bind(SYMBOL)
    .execute(pool)
    .await
    .expect("種入股票主檔");
    PgFinancialRepository::new()
        .batch_save_financial_statements(&[zero_1991_q1()])
        .await
        .expect("種入 1991-Q1 財報");
    sqlx::query(
        "INSERT INTO config (key, val) VALUES ('estimate-date', '1991-06-20')
         ON CONFLICT (key) DO UPDATE SET val = EXCLUDED.val",
    )
    .execute(pool)
    .await
    .expect("設定估價日期");

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/quote/{SYMBOL}/profile")))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(PROFILE_PAGE.replace("2025 Q3", "1991 Q1")),
        )
        .mount(&server)
        .await;
    let uri = server.uri();

    let result = with_origins(&[(YAHOO, &uri), (NSTOCK, &uri)], execute_at(now())).await;
    let filled: Vec<(rust_decimal::Decimal, rust_decimal::Decimal)> = sqlx::query_as(
        "SELECT return_on_equity, return_on_assets FROM financial_statement
         WHERE security_code = $1 AND year = 1991 AND quarter = 'Q1'",
    )
    .bind(SYMBOL)
    .fetch_all(pool)
    .await
    .expect("讀回補值後財報");
    let cached = CLIENT
        .get_bool(&format!("FinancialStatement:{SYMBOL}-1991-Q1"))
        .await;
    cleanup_1991(estimate_date.as_deref()).await;

    result.expect("季報補值排程完成");
    assert_eq!(filled.len(), 1, "{filled:?}");
    let (roe, roa) = filled[0];
    assert!(
        !roe.is_zero() && !roa.is_zero(),
        "ROE／ROA 由 Yahoo 補上：{roe} {roa}"
    );
    assert!(cached.expect("讀取成功快取"), "補到的列要記成功快取");
}

/// 目標季度沒有待補的列時不查 Yahoo，也不需要估價日期設定。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn execute_at_without_candidates_skips_recalculation() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() || CLIENT.ping().await.is_err() {
        println!("跳過 execute_at_without_candidates_skips_recalculation：無資料庫或 Redis 連接");
        return;
    }
    let server = MockServer::start().await;
    let uri = server.uri();

    with_origins(&[(YAHOO, &uri), (NSTOCK, &uri)], execute_at(now()))
        .await
        .expect("沒有待補的列不算錯誤");

    let yahoo_requests = server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|request| request.url.path().ends_with("/profile"))
        .count();
    assert_eq!(yahoo_requests, 0, "沒有待補的列不查 Yahoo");
}

/// 排程入口以今天決定目標季度；外部來源都換成 mock（Yahoo、nstock 一律 404），不會寫入任何財報。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn execute_uses_today_targets() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() || CLIENT.ping().await.is_err() {
        println!("跳過 execute_uses_today_targets：無資料庫或 Redis 連接");
        return;
    }
    let server = MockServer::start().await;
    let uri = server.uri();

    with_origins(&[(YAHOO, &uri), (NSTOCK, &uri)], execute())
        .await
        .expect("排程入口完成");
}
