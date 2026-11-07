//! 連資料庫的整輪與單檔補抓：多次配息快取、Yahoo（mock）股利頁寫入股利表。

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::core::util::http::mock_origin::with_origins;
use crate::infra::database;

/// 測試資料庫可用時回傳連線池；不可用時印出略過訊息。
async fn pool_or_skip(test: &str) -> Option<&'static sqlx::PgPool> {
    dotenvy::dotenv().ok();
    let pool = database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 {test}：無資料庫連接");
        return None;
    }
    Some(pool)
}

async fn delete_dividends(pool: &sqlx::PgPool, symbol: &str) {
    sqlx::query("DELETE FROM dividend WHERE security_code = $1")
        .bind(symbol)
        .execute(pool)
        .await
        .expect("清理測試股利");
}

/// 整輪流程：指定年度已有半年配的股票會放進候選與快取；收到關機請求就在第一檔之前停下。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn refresh_year_caches_multiple_dividends_of_the_year() {
    const SYMBOL: &str = "79979DW";
    let Some(pool) = pool_or_skip("refresh_year_caches_multiple_dividends_of_the_year").await
    else {
        return;
    };
    delete_dividends(pool, SYMBOL).await;
    sqlx::query(r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2", payable_date1, payable_date2) VALUES ($1, 2099, 2098, 'H1', 1.5, 0, 1.5, '-', '-', '-', '-')"#)
        .bind(SYMBOL)
        .execute(pool)
        .await
        .expect("寫入測試半年配");

    let result = refresh_year(2099, || true).await;

    delete_dividends(pool, SYMBOL).await;
    result.expect("提早結束不是錯誤");
}

/// 單檔補抓：Yahoo 股利頁（mock，內容為 2753 的實際頁面）只寫入指定年度與前一年度，
/// 半年配寫成各期列並重算年度彙總。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn backfill_recent_dividends_for_stock_saves_recent_yahoo_rows() {
    const SYMBOL: &str = "79979DX";
    let Some(pool) =
        pool_or_skip("backfill_recent_dividends_for_stock_saves_recent_yahoo_rows").await
    else {
        return;
    };
    delete_dividends(pool, SYMBOL).await;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/quote/{SYMBOL}/dividend")))
        .respond_with(ResponseTemplate::new(200).set_body_string(include_str!(
            "../../../../../infra/crawler/yahoo/testdata/dividend_2753.html"
        )))
        .mount(&server)
        .await;
    let uri = server.uri();

    let result = with_origins(
        &[("https://tw.stock.yahoo.com", uri.as_str())],
        backfill_recent_dividends_for_stock(2026, SYMBOL, &HashMap::new()),
    )
    .await;
    let rows: Vec<(i32, String)> = sqlx::query_as(
        "SELECT year_of_dividend, quarter FROM dividend WHERE security_code = $1 ORDER BY 1, 2",
    )
    .bind(SYMBOL)
    .fetch_all(pool)
    .await
    .expect("讀回測試股利");

    delete_dividends(pool, SYMBOL).await;
    result.expect("mock 頁面可解析");
    assert!(!rows.is_empty(), "應寫入近兩年的股利");
    assert!(
        rows.iter().all(|(year, _)| (2025..=2026).contains(year)),
        "只寫指定年度與前一年度：{rows:?}"
    );
    assert!(
        rows.iter().any(|(_, quarter)| quarter.starts_with('H')),
        "半年配寫成各期列：{rows:?}"
    );
}
