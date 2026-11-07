//! 興櫃每股淨值回補：只寫回有變動的股票，排程入口以 mock TPEx 執行。

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::core::util::http::mock_origin::with_origins;
use crate::infra::cache::SHARE;

/// 每股淨值會改變、不變、主檔沒有的三個假代號。
const CHANGED: &str = "79979NA";
const UNCHANGED: &str = "79979NB";
const UNKNOWN: &str = "79979NC";

async fn database_unavailable(test: &str) -> bool {
    dotenvy::dotenv().ok();
    if crate::infra::database::ping().await.is_err() {
        println!("跳過 {test}：無資料庫連接");
        return true;
    }
    false
}

async fn cleanup() {
    let symbols = [CHANGED, UNCHANGED, UNKNOWN];
    sqlx::query("DELETE FROM stocks WHERE stock_symbol = ANY($1)")
        .bind(&symbols[..])
        .execute(crate::infra::database::get_connection())
        .await
        .expect("清除測試股票");
    if let Ok(mut stocks) = SHARE.stocks.write() {
        for symbol in symbols {
            stocks.remove(symbol);
        }
    }
}

async fn seed(symbol: &str, net_asset_value: Decimal) {
    sqlx::query(
        r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, "SuspendListing", net_asset_value_per_share)
           VALUES ($1, '興櫃淨值測試', $1, 5, false, $2)"#,
    )
    .bind(symbol)
    .bind(net_asset_value)
    .execute(crate::infra::database::get_connection())
    .await
    .expect("種入測試股票");
}

async fn net_asset_value(symbol: &str) -> Decimal {
    sqlx::query_scalar("SELECT net_asset_value_per_share FROM stocks WHERE stock_symbol = $1")
        .bind(symbol)
        .fetch_one(crate::infra::database::get_connection())
        .await
        .expect("讀回每股淨值")
}

/// 只有淨值不同的那檔寫回；淨值相同與主檔沒有的代號都略過。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn update_changed_writes_only_different_values() {
    if database_unavailable("update_changed_writes_only_different_values").await {
        return;
    }
    cleanup().await;
    seed(CHANGED, dec!(10)).await;
    seed(UNCHANGED, dec!(15)).await;

    let items = [
        Emerging::new(CHANGED.to_owned(), dec!(12.34)),
        Emerging::new(UNCHANGED.to_owned(), dec!(15)),
        Emerging::new(UNKNOWN.to_owned(), dec!(20)),
    ];
    let updated = update_changed(&items).await;
    let values = (
        net_asset_value(CHANGED).await,
        net_asset_value(UNCHANGED).await,
    );
    cleanup().await;

    assert_eq!(updated.expect("比對完成"), 1);
    assert_eq!(values, (dec!(12.34), dec!(15)));
}

/// 排程入口：週末直接結束；平日抓 mock 的興櫃名冊（沒有資料列）後不寫任何資料。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn execute_fetches_emerging_list_on_weekdays() {
    if database_unavailable("execute_fetches_emerging_list_on_weekdays").await {
        return;
    }
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/web/regular_emerging/corporateInfo/emerging/emerging_stock.php",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"<html><body><table id="company_list"><tbody></tbody></table></body></html>"#,
        ))
        .mount(&server)
        .await;
    let uri = server.uri();

    let result = with_origins(&[("https://www.tpex.org.tw", &uri)], execute()).await;

    assert!(result.is_ok(), "{result:?}");
}
