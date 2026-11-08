//! 來源抓取（mock 證交所、櫃買）與排程入口（mock 來源＋臨時資料庫）。

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

/// 排程入口：四個來源都有資料時讀當天成交量、挑出要通知的持股；Telegram 也導向 mock server，
/// 不論資料庫裡有沒有持股都不會真的送出訊息。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn execute_runs_against_mocked_sources() {
    dotenvy::dotenv().ok();
    if sqlx::query("SELECT 1")
        .execute(database::get_connection())
        .await
        .is_err()
    {
        println!("跳過 execute_runs_against_mocked_sources：無資料庫連接");
        return;
    }
    let server = MockServer::start().await;
    serve(&server, "/rwd/zh/fund/T86", T86_FIXTURE).await;
    serve(&server, "/rwd/zh/marginTrading/MI_MARGN", MI_MARGN_FIXTURE).await;
    serve(
        &server,
        "/web/stock/3insti/daily_trade/3itrade_hedge_result.php",
        TPEX_INSTITUTIONAL_FIXTURE,
    )
    .await;
    let uri = server.uri();

    with_origins(
        &[
            ("https://www.twse.com.tw", &uri),
            ("https://www.tpex.org.tw", &uri),
            ("https://api.telegram.org", &uri),
        ],
        execute(),
    )
    .await
    .expect("持股籌碼通知完成");
}
