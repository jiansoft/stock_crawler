//! 以本機 mock server 取代交易所（G15）：休市判斷與收盤匯總的「當天沒有行情」分支。

use chrono::TimeZone;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::core::util::http::mock_origin::with_origins;

const TWSE: &str = "https://www.twse.com.tw";
const TPEX: &str = "https://www.tpex.org.tw";
const HOLIDAY_PATH: &str = "/rwd/zh/holidaySchedule/holidaySchedule";

/// 台北時間 2026 年某月某日中午。
fn noon(month: u32, day: u32) -> DateTime<Local> {
    let naive = NaiveDate::from_ymd_opt(2026, month, day)
        .and_then(|date| date.and_hms_opt(12, 0, 0))
        .expect("有效日期");
    Local
        .from_local_datetime(&naive)
        .single()
        .expect("本地時間沒有歧義")
}

/// 只有 2026-10-09（五）一天休市的休市日清單。
async fn holiday_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(HOLIDAY_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "stat": "OK",
            "date": "20260101",
            "data": [
                ["2026-10-09", "國慶日補假", "依規定放假1日。"],
                ["2026-01-05", "開始交易", "國曆新年開始交易日。"]
            ],
            "queryYear": 2026,
            "total": 2
        })))
        .mount(&server)
        .await;
    server
}

/// 週末不查清單；平日依交易所休市日判斷。
#[tokio::test]
async fn is_market_closed_uses_weekend_and_holiday_schedule() {
    let server = holiday_server().await;
    let (holiday, trading_day, weekend) = with_origins(&[(TWSE, &server.uri())], async {
        tokio::join!(
            is_market_closed(noon(10, 9)),
            is_market_closed(noon(10, 8)),
            is_market_closed(noon(10, 10)),
        )
    })
    .await;

    assert!(holiday, "交易所公告的休市日");
    assert!(!trading_day, "清單上沒有的平日照常開市");
    assert!(weekend, "週六");
    let requests = server.received_requests().await.unwrap_or_default();
    assert_eq!(requests.len(), 2, "週末不查休市日清單");
}

/// 休市日清單抓不到時視為開市，交給收盤匯總「0 筆即略過」處理。
#[tokio::test]
async fn is_market_closed_treats_schedule_failure_as_open() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(HOLIDAY_PATH))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let closed = with_origins(&[(TWSE, &server.uri())], is_market_closed(noon(10, 8))).await;

    assert!(!closed);
}

/// 上市、上櫃當天都沒有行情時，收盤匯總直接結束，不碰資料庫。
#[tokio::test]
async fn aggregate_skips_when_both_markets_have_no_quotes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/exchangeReport/MI_INDEX"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "stat": "很抱歉，沒有符合條件的資料!" })),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/openapi/v1/tpex_mainboard_peratio_analysis"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/web/stock/aftertrading/otc_quotes_no1430/stk_wn1430_result.php",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "tables": [] })))
        .mount(&server)
        .await;

    let date = NaiveDate::from_ymd_opt(2026, 10, 9).expect("有效日期");
    let result = with_origins(
        &[(TWSE, &server.uri()), (TPEX, &server.uri())],
        aggregate(date),
    )
    .await;

    result.expect("沒有行情時略過，不回傳錯誤");
    let requests = server.received_requests().await.unwrap_or_default();
    assert_eq!(requests.len(), 3, "上市行情、上櫃本益比與上櫃行情各一次");
}
