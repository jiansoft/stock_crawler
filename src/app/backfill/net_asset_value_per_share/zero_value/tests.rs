//! 淨值為零修補（05:10 排程）：以本機 mock server 取代 Yahoo（G15），用假代號跑完整流程。

use rust_decimal::Decimal;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::core::util::http::mock_origin::with_origins;
use crate::infra::database;
use crate::infra::nosql::redis::CLIENT;

const YAHOO: &str = "https://tw.stock.yahoo.com";
/// Yahoo 個股 profile 頁 fixture，每股淨值 158.9。
const PROFILE_PAGE: &str =
    include_str!("../../../../infra/crawler/yahoo/testdata/profile_page.html");

/// Yahoo 回有效 profile → 更新成 158.9。
const FILLED: &str = "79952";
/// Yahoo 回 404（多為已下市）→ 設 30 天略過快取，淨值維持 0。
const NOT_FOUND: &str = "79953";
/// 已有略過快取 → 不打 Yahoo。
const CACHED_SKIP: &str = "79954";
/// Yahoo 的每股淨值也是 0 → 不寫入。
const STILL_ZERO: &str = "79955";
/// 頁面存在但解析不到區塊（改版）→ 記 error、不設略過快取。
const BROKEN_PAGE: &str = "79956";
/// 特別股（代號含英文字母）→ 直接略過。
const PREFERENCE: &str = "79958A";
/// TDR（91 開頭）→ 直接略過。
const TDR: &str = "91998";

const SYMBOLS: [&str; 7] = [
    FILLED,
    NOT_FOUND,
    CACHED_SKIP,
    STILL_ZERO,
    BROKEN_PAGE,
    PREFERENCE,
    TDR,
];

async fn cleanup() {
    let pool = database::get_connection();
    for symbol in SYMBOLS {
        sqlx::query("DELETE FROM stocks WHERE stock_symbol = $1")
            .bind(symbol)
            .execute(pool)
            .await
            .unwrap_or_else(|why| panic!("清除 {symbol}: {why}"));
        let _ = CLIENT
            .delete(&profile::no_valid_data_cache_key(symbol))
            .await;
    }
}

async fn net_asset_value(symbol: &str) -> Decimal {
    sqlx::query_scalar("SELECT net_asset_value_per_share FROM stocks WHERE stock_symbol = $1")
        .bind(symbol)
        .fetch_one(database::get_connection())
        .await
        .unwrap_or_else(|why| panic!("讀回 {symbol} 淨值: {why}"))
}

async fn mount_profile(server: &MockServer, symbol: &str, response: ResponseTemplate, calls: u64) {
    Mock::given(method("GET"))
        .and(path(format!("/quote/{symbol}/profile")))
        .respond_with(response)
        .expect(calls)
        .mount(server)
        .await;
}

/// 每股淨值為 0 的上市櫃股票逐檔問 Yahoo：有值就更新，404 設略過快取，
/// 已有略過快取、特別股與 TDR 不查，Yahoo 也是 0 或頁面改版時不寫入。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn execute_fills_zero_net_asset_values_from_yahoo_profile() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() || CLIENT.ping().await.is_err() {
        println!(
            "跳過 execute_fills_zero_net_asset_values_from_yahoo_profile：無資料庫或 Redis 連接"
        );
        return;
    }
    cleanup().await;
    let pool = database::get_connection();
    for symbol in SYMBOLS {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, "SuspendListing", net_asset_value_per_share)
               VALUES ($1, '淨值修補測試', $1, 2, false, 0)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .unwrap_or_else(|why| panic!("種入 {symbol}: {why}"));
    }
    CLIENT
        .set(profile::no_valid_data_cache_key(CACHED_SKIP), true, 600)
        .await
        .expect("預先設定略過快取");

    let server = MockServer::start().await;
    mount_profile(
        &server,
        FILLED,
        ResponseTemplate::new(200).set_body_string(PROFILE_PAGE),
        1,
    )
    .await;
    mount_profile(&server, NOT_FOUND, ResponseTemplate::new(404), 1).await;
    mount_profile(
        &server,
        STILL_ZERO,
        ResponseTemplate::new(200)
            .set_body_string(PROFILE_PAGE.replace("<div>158.9</div>", "<div>0</div>")),
        1,
    )
    .await;
    mount_profile(
        &server,
        BROKEN_PAGE,
        ResponseTemplate::new(200).set_body_string("<html><body>改版了</body></html>"),
        1,
    )
    .await;
    for symbol in [CACHED_SKIP, PREFERENCE, TDR] {
        mount_profile(&server, symbol, ResponseTemplate::new(200), 0).await;
    }
    let uri = server.uri();

    let result = with_origins(&[(YAHOO, &uri)], execute()).await;
    let mut values = Vec::with_capacity(SYMBOLS.len());
    for symbol in SYMBOLS {
        values.push((symbol, net_asset_value(symbol).await));
    }
    let not_found_skipped = CLIENT
        .get_bool(&profile::no_valid_data_cache_key(NOT_FOUND))
        .await;
    let broken_skipped = CLIENT
        .get_bool(&profile::no_valid_data_cache_key(BROKEN_PAGE))
        .await;
    cleanup().await;

    result.expect("淨值修補排程完成");
    let expected: Vec<(&str, Decimal)> = SYMBOLS
        .iter()
        .map(|&symbol| {
            let value = if symbol == FILLED {
                rust_decimal_macros::dec!(158.9)
            } else {
                Decimal::ZERO
            };
            (symbol, value)
        })
        .collect();
    assert_eq!(values, expected, "只有 Yahoo 回有效淨值的股票被更新");
    assert!(
        not_found_skipped.expect("讀取 404 略過快取"),
        "404 要設略過快取，之後不再每天打 Yahoo"
    );
    assert!(
        !broken_skipped.expect("讀取改版略過快取"),
        "解析失敗不是無資料，不應設略過快取"
    );
    // MockServer drop 時驗證呼叫次數：已有略過快取、特別股與 TDR 都不該打 Yahoo。
}

#[tokio::test]
#[ignore = "live test：連線正式資料庫與 Yahoo，需要時手動執行"]
async fn test_execute() {
    dotenvy::dotenv().ok();
    if let Err(why) = execute().await {
        tracing::debug!("Failed to execute because {:?}", why);
    }
}
