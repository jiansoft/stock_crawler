//! 以本機 mock server 取代交易所、MOPS、MoneyDJ 與 Yahoo（G15），跑一次完整的除權息公告掃描。

use rust_decimal_macros::dec;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::core::util::http::mock_origin::with_origins;

/// 第一階段可由 MOPS 金額吻合判定期別的假代號。
const MATCHED: &str = "79991";
/// MOPS 沒有資料、Yahoo 也查無頁面，只能列為無法判定的假代號。
const UNRESOLVED: &str = "79992";

const MONEYDJ_FIXTURE: &str =
    include_str!("../../../../infra/crawler/moneydj/testdata/dividend_schedule_zey.html");

/// 上市預告表：兩檔都在 2026-09-07（民國 1150907）除息。
fn twt48u() -> serde_json::Value {
    let row = |code: &str, cash: &str| {
        serde_json::json!({
            "Date": "1150907", "Code": code, "Name": code, "Exdividend": "息",
            "StockDividendRatio": "", "SubscriptionRatio": "", "SubscriptionPricePerShare": "",
            "CashDividend": cash, "SharesOffered": "", "SharesEmpOwner": "",
            "SharesholderOwner": "", "StockHoldingRatio": ""
        })
    };
    serde_json::json!([row(MATCHED, "1.500000"), row(UNRESOLVED, "0.300000")])
}

/// MOPS 股利分派：只有 [`MATCHED`] 的 114 年度盈餘配息 1.5 元。
fn t187ap45() -> serde_json::Value {
    serde_json::json!([{
        "出表日期": "1150905", "公司代號": MATCHED, "公司名稱": MATCHED,
        "決議（擬議）進度": "股東會確認", "股利年度": "114", "股利所屬年(季)度": "年度",
        "股利所屬期間": "1140101~1141231", "期別": "1",
        "股東配發-盈餘分配之現金股利(元/股)": "1.50000000",
        "股東配發-法定盈餘公積發放之現金(元/股)": "0.0",
        "股東配發-資本公積發放之現金(元/股)": "0.0",
        "股東配發-盈餘轉增資配股(元/股)": "0.0",
        "股東配發-法定盈餘公積轉增資配股(元/股)": "0.0",
        "股東配發-資本公積轉增資配股(元/股)": "0.0"
    }])
}

/// 掛上五個來源：上市預告表、上櫃預告表（空）、MOPS 分派、MoneyDJ 政策表（Big5）、Yahoo（404）。
async fn mount_sources(server: &MockServer) {
    let json = |body: serde_json::Value| ResponseTemplate::new(200).set_body_json(body);
    Mock::given(method("GET"))
        .and(path("/v1/exchangeReport/TWT48U_ALL"))
        .respond_with(json(twt48u()))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/openapi/v1/tpex_exright_prepost"))
        .respond_with(json(serde_json::json!([])))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/opendata/t187ap45_L"))
        .respond_with(json(t187ap45()))
        .mount(server)
        .await;
    let (big5, _, _) = encoding_rs::BIG5.encode(MONEYDJ_FIXTURE);
    Mock::given(method("GET"))
        .and(path("/Z/ZE/ZEY/ZEY.djhtm"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(big5.into_owned(), "text/html"))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/quote/{UNRESOLVED}/dividend")))
        .respond_with(ResponseTemplate::new(404))
        .mount(server)
        .await;
}

/// 資料庫沒有的事件：MOPS 金額吻合的那檔新增一列（帶除息日、不猜發放日），
/// 判不出期別的那檔問過 Yahoo 後只計數、絕不寫入。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn scan_inserts_matched_event_and_skips_unresolved() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 scan_inserts_matched_event_and_skips_unresolved：無資料庫連接");
        return;
    }
    let symbols = vec![MATCHED.to_string(), UNRESOLVED.to_string()];
    let cleanup = || async {
        sqlx::query("DELETE FROM dividend WHERE security_code = ANY($1)")
            .bind(&symbols)
            .execute(pool)
            .await
            .expect("清除測試股利");
    };
    cleanup().await;

    let server = MockServer::start().await;
    mount_sources(&server).await;
    let uri = server.uri();
    let outcome = with_origins(
        &[
            ("https://openapi.twse.com.tw", &uri),
            ("https://www.tpex.org.tw", &uri),
            ("https://www.moneydj.com", &uri),
            ("https://tw.stock.yahoo.com", &uri),
        ],
        scan_ex_dividend_announcements(),
    )
    .await;
    let rows: Vec<(String, String, rust_decimal::Decimal, String)> = sqlx::query_as(
        r#"SELECT security_code, "ex-dividend_date1", cash_dividend, payable_date1 FROM dividend WHERE security_code = ANY($1)"#,
    )
    .bind(&symbols)
    .fetch_all(pool)
    .await
    .expect("讀回股利");
    cleanup().await;

    let outcome = outcome.expect("掃描成功");
    assert_eq!(
        (outcome.inserted, outcome.updated, outcome.unresolved),
        (1, 0, 1)
    );
    assert_eq!(
        rows,
        vec![(
            MATCHED.to_string(),
            "2026-09-07".to_string(),
            dec!(1.5),
            UNANNOUNCED_DATE.to_string()
        )]
    );
}

/// 上市與上櫃預告表都抓不到時整個掃描失敗，不碰資料庫。
#[tokio::test]
async fn scan_fails_when_both_announcement_tables_are_unavailable() {
    let server = MockServer::start().await;
    let uri = server.uri();
    let error = with_origins(
        &[
            ("https://openapi.twse.com.tw", &uri),
            ("https://www.tpex.org.tw", &uri),
            ("https://www.moneydj.com", &uri),
        ],
        scan_ex_dividend_announcements(),
    )
    .await
    .expect_err("兩個預告表都是 404");

    assert!(error.to_string().contains("都無法取得"), "{error:#}");
}
