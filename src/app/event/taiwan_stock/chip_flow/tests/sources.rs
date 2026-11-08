//! 來源抓取（mock 證交所、櫃買）與成交量讀取（臨時資料庫）。

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::core::util::http::mock_origin::with_origins;
use crate::infra::database;

const T86_FIXTURE: &str =
    include_str!("../../../../../infra/crawler/twse/testdata/institutional_t86.json");
const MI_MARGN_FIXTURE: &str =
    include_str!("../../../../../infra/crawler/twse/testdata/margin_mi_margn.json");
const TPEX_INSTITUTIONAL_FIXTURE: &str =
    include_str!("../../../../../infra/crawler/tpex/testdata/institutional_3itrade.json");

async fn serve(server: &MockServer, route: &str, body: &str) {
    Mock::given(method("GET"))
        .and(path(route))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body.to_owned(), "application/json"))
        .mount(server)
        .await;
}

/// 上市、上櫃各自合併成一張表；上櫃融資融券來源失敗（404）只少那一份，其餘照常。
#[tokio::test]
async fn fetch_sources_merges_markets_and_tolerates_a_failed_source() {
    let server = MockServer::start().await;
    serve(&server, "/rwd/zh/fund/T86", T86_FIXTURE).await;
    serve(&server, "/rwd/zh/marginTrading/MI_MARGN", MI_MARGN_FIXTURE).await;
    serve(
        &server,
        "/web/stock/3insti/daily_trade/3itrade_hedge_result.php",
        TPEX_INSTITUTIONAL_FIXTURE,
    )
    .await;
    // 上櫃融資融券沒有掛路由：mock server 回 404。
    let uri = server.uri();

    let (flows, margins) = with_origins(
        &[
            ("https://www.twse.com.tw", &uri),
            ("https://www.tpex.org.tw", &uri),
        ],
        fetch_sources(day(5)),
    )
    .await;

    assert!(flows.contains_key("2330"), "上市三大法人");
    assert!(flows.contains_key("5347"), "上櫃三大法人");
    assert!(margins.contains_key("2330"), "上市融資融券");
    assert!(
        !margins.contains_key("5347"),
        "上櫃融資融券來源失敗，不應有資料"
    );
}

/// 讀當天成交量（股）；沒有日線的日子回空表。假代號結束時刪除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn load_volumes_reads_trading_volume_of_the_day() {
    dotenvy::dotenv().ok();
    let pool = database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 load_volumes_reads_trading_volume_of_the_day：無資料庫連接");
        return;
    }
    let symbol = "79976V";
    let date = NaiveDate::from_ymd_opt(1999, 3, 10).expect("合法日期");
    let cleanup = || async {
        sqlx::query(r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = $1"#)
            .bind(symbol)
            .execute(pool)
            .await
            .expect("清除測試日線");
    };
    cleanup().await;
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "TradingVolume")
           VALUES ($1, $2, 10, 123456)"#,
    )
    .bind(date)
    .bind(symbol)
    .execute(pool)
    .await
    .expect("插入日線");

    let volumes = load_volumes(date).await;
    let next_day = load_volumes(date.succ_opt().expect("隔天")).await;
    cleanup().await;

    assert_eq!(volumes.get(symbol), Some(&123_456));
    assert!(!next_day.contains_key(symbol));
}
