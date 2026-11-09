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

/// 收盤匯總測試日（1991-03-05，週二）；所有以日期為鍵的衍生表都只清這一天。
const AGGREGATE_DATE: (i32, u32, u32) = (1991, 3, 5);
/// 測試用的上市、上櫃假代號。
const LISTED: &str = "79979CA";
const OTC: &str = "79979CB";

/// TWSE MI_INDEX：只有一檔上市股的收盤行情表。
fn listed_quotes() -> serde_json::Value {
    serde_json::json!({
        "stat": "OK",
        "tables": [{
            "title": "每日收盤行情(全部(不含權證、牛熊證))",
            "fields": [
                "證券代號", "證券名稱", "成交股數", "成交筆數", "成交金額", "開盤價", "最高價",
                "最低價", "收盤價", "漲跌(+/-)", "漲跌價差", "最後揭示買價", "最後揭示買量",
                "最後揭示賣價", "最後揭示賣量", "本益比"
            ],
            "data": [[
                LISTED, "測試上市", "10,000", "100", "5,000,000", "500.00", "505.00", "499.00",
                "502.00", "+", "2.00", "502.00", "10", "503.00", "20", "15.5"
            ]]
        }]
    })
}

/// TPEx 上櫃行情：只有一檔上櫃股。
fn otc_quotes() -> serde_json::Value {
    serde_json::json!({
        "tables": [{
            "data": [[
                OTC, "測試上櫃", "100.00", "1.50", "98.50", "101.00", "98.00", "10,000",
                "1,000,000", "500", "100.00", "10", "100.50", "20"
            ]]
        }]
    })
}

/// 清掉收盤匯總在測試日寫入的所有資料；`last-closing-day` 還原成測試前的值。
async fn cleanup_aggregate(date: NaiveDate, last_closing_day: Option<&str>) {
    let pool = crate::infra::database::get_connection();
    for sql in [
        r#"DELETE FROM "DailyQuotes" WHERE "Date" = $1"#,
        "DELETE FROM estimate WHERE date = $1",
        "DELETE FROM yield_rank WHERE date = $1",
        "DELETE FROM daily_technical_signal WHERE date = $1",
        "DELETE FROM daily_forward_return WHERE date = $1",
        "DELETE FROM daily_market_forward_return WHERE date = $1",
        "DELETE FROM daily_relative_strength WHERE date = $1",
        "DELETE FROM daily_money_history_detail_more WHERE date = $1",
        "DELETE FROM daily_money_history_detail WHERE date = $1",
        "DELETE FROM daily_money_history_member WHERE date = $1",
        "DELETE FROM daily_money_history WHERE date = $1",
        "DELETE FROM daily_stock_price_stats WHERE date = $1",
    ] {
        sqlx::query(sql)
            .bind(date)
            .execute(pool)
            .await
            .unwrap_or_else(|why| panic!("{sql}: {why}"));
    }
    sqlx::query("DELETE FROM quote_history_record WHERE security_code = ANY($1)")
        .bind([LISTED, OTC])
        .execute(pool)
        .await
        .expect("清除測試股價歷史紀錄");
    match last_closing_day {
        Some(val) => sqlx::query("UPDATE config SET val = $2 WHERE key = $1").bind(val),
        None => sqlx::query("DELETE FROM config WHERE key = $1"),
    }
    .bind("last-closing-day")
    .execute(pool)
    .await
    .expect("還原最後收盤日設定");
}

/// 上市、上櫃都有行情時跑完整收盤匯總：當日報價原子寫入兩市場，後續計算都不中斷。
///
/// 除權息基準價來源沒有 mock（404）：只記錯誤，不影響匯總。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn aggregate_writes_both_markets_and_runs_calculations() {
    dotenvy::dotenv().ok();
    if crate::infra::database::ping().await.is_err() {
        println!("跳過 aggregate_writes_both_markets_and_runs_calculations：無資料庫連接");
        return;
    }
    let pool = crate::infra::database::get_connection();
    let (year, month, day) = AGGREGATE_DATE;
    let date = NaiveDate::from_ymd_opt(year, month, day).expect("有效日期");
    let last_closing_day: Option<String> =
        sqlx::query_scalar("SELECT val FROM config WHERE key = 'last-closing-day'")
            .fetch_optional(pool)
            .await
            .expect("讀取最後收盤日設定");
    cleanup_aggregate(date, last_closing_day.as_deref()).await;

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/exchangeReport/MI_INDEX"))
        .respond_with(ResponseTemplate::new(200).set_body_json(listed_quotes()))
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
        .respond_with(ResponseTemplate::new(200).set_body_json(otc_quotes()))
        .mount(&server)
        .await;

    let result = with_origins(
        &[(TWSE, &server.uri()), (TPEX, &server.uri())],
        aggregate(date),
    )
    .await;

    let quotes: Vec<(String, rust_decimal::Decimal)> = sqlx::query_as(
        r#"SELECT stock_symbol, "ClosingPrice" FROM "DailyQuotes"
           WHERE "Date" = $1 AND stock_symbol = ANY($2) ORDER BY stock_symbol"#,
    )
    .bind(date)
    .bind([LISTED, OTC])
    .fetch_all(pool)
    .await
    .expect("讀回測試日報價");
    cleanup_aggregate(date, last_closing_day.as_deref()).await;

    result.expect("收盤匯總完成");
    assert_eq!(
        quotes,
        vec![
            (LISTED.to_string(), rust_decimal::Decimal::new(502, 0)),
            (OTC.to_string(), rust_decimal::Decimal::new(100, 0)),
        ],
        "上市、上櫃當日報價都要寫入"
    );
    let paths: Vec<String> = server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .map(|request| request.url.path().to_string())
        .collect();
    assert!(
        paths.iter().any(|path| path.ends_with("TWT49U")),
        "有行情才查除權息基準價：{paths:?}"
    );
}

/// 收盤匯總與盤後完整性檢查的本體：週末不查休市日清單、休市日查完清單就結束，都不碰行情來源與資料庫。
#[tokio::test]
async fn run_and_ensure_complete_skip_closed_days() {
    let server = holiday_server().await;
    let uri = server.uri();

    with_origins(&[(TWSE, &uri)], async {
        run(noon(10, 10)).await.expect("週六略過");
        run(noon(10, 9)).await.expect("休市日略過");
        ensure_complete_on(noon(10, 10)).await.expect("週六略過");
        ensure_complete_on(noon(10, 9)).await.expect("休市日略過");
    })
    .await;

    let requests = server.received_requests().await.unwrap_or_default();
    assert!(
        requests
            .iter()
            .all(|request| request.url.path() == HOLIDAY_PATH),
        "休市時只會查休市日清單"
    );
}

/// 兩個排程入口以現在時間執行：把今天設為休市日，不論哪天跑都只查休市日清單就結束。
#[tokio::test]
async fn entries_use_current_time() {
    let today = Local::now().date_naive();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(HOLIDAY_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "stat": "OK",
            "date": format!("{}0101", today.year()),
            "data": [[today.format("%Y-%m-%d").to_string(), "測試休市", ""]],
            "queryYear": today.year(),
            "total": 1
        })))
        .mount(&server)
        .await;
    let uri = server.uri();

    with_origins(&[(TWSE, &uri), (TPEX, &uri)], async {
        execute().await.expect("收盤匯總略過");
        ensure_complete().await.expect("完整性檢查略過");
    })
    .await;

    let requests = server.received_requests().await.unwrap_or_default();
    assert!(
        requests
            .iter()
            .all(|request| request.url.path() == HOLIDAY_PATH),
        "休市時只會查休市日清單"
    );
}
