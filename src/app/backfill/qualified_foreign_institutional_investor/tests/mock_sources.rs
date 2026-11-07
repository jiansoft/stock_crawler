//! 以本機 mock server 取代證交所與櫃買（G16）：抓取後以批次寫回股票主檔（P13）。

use chrono::TimeZone;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::core::util::http::mock_origin::with_origins;

/// 上市假代號：外資持股會從 100 股變成 400 股。
const LISTED: &str = "79993QF";
/// 上櫃假代號：外資持股會從 100 股變成 22,154,306 股。
const OTC: &str = "79994QF";

/// 上市 MI_QFIIS 的一列（12 欄：代號、名稱、ISIN、發行股數、尚可投資、持有股數、
/// 尚可投資比率、持股比率、上限、陸資上限、異動原因、日期）。
fn mi_qfiis() -> serde_json::Value {
    serde_json::json!({
        "stat": "OK", "date": "20260930", "selectType": "ALLBUT0999", "total": 1,
        "fields": [], "data": [[
            LISTED, "測試上市", "TW0000000000", "1,000", "100", "400",
            "10.00", "40.00", "100.00", "100.00", "", "1150930"
        ]]
    })
}

/// 櫃買 QFII 排行表，只有一檔；資料日取最外層的 `date`。
fn tpex_qfii() -> serde_json::Value {
    serde_json::json!({
        "stat": "ok",
        "date": "20260930",
        "tables": [{
            "title": "櫃檯買賣股票僑外資及陸資持股比例排行表",
            "date": "115/09/30", "totalCount": 1,
            "fields": [
                "排行", "代號", "名稱", "發行股數(A)", "僑外資及陸資尚可投資股數B=A*F-C",
                "僑外資及陸資持有股數(C)", "僑外資及陸資尚可投資比率(D=B/A)",
                "僑外資及陸資持股比率(E=C/A)", "法令投資上限比率(F)", "備註"
            ],
            "data": [[
                "1", OTC, "測試上櫃", "25,219,056", "3,064,750", "22,154,306",
                "12.15%", "87.84%", "100%", ""
            ]]
        }]
    })
}

/// 上市以 2026-09-30 15:00（台北）查詢，上櫃讀櫃買回應的日期；兩邊都寫回主檔與快取。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn listed_and_otc_fetch_then_batch_update_stocks() {
    dotenvy::dotenv().ok();
    if crate::infra::database::ping().await.is_err() {
        println!("跳過 listed_and_otc_fetch_then_batch_update_stocks：無資料庫連接");
        return;
    }
    let pool = crate::infra::database::get_connection();
    let symbols = vec![LISTED.to_string(), OTC.to_string()];
    let cleanup = || async {
        let _ = sqlx::query("DELETE FROM stocks WHERE stock_symbol = ANY($1)")
            .bind(&symbols)
            .execute(pool)
            .await;
        if let Ok(mut cache) = SHARE.stocks.write() {
            cache.remove(LISTED);
            cache.remove(OTC);
        }
    };
    cleanup().await;
    for (symbol, market) in [(LISTED, 2), (OTC, 4)] {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, "SuspendListing", issued_share, qfii_shares_held, qfii_share_holding_percentage)
               VALUES ($1, $1, $1, $2, false, 1000, 100, 10)"#,
        )
        .bind(symbol)
        .bind(market)
        .execute(pool)
        .await
        .expect("種入股票主檔");
    }

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rwd/zh/fund/MI_QFIIS"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mi_qfiis()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/www/zh-tw/insti/qfii"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tpex_qfii()))
        .mount(&server)
        .await;
    let taipei = FixedOffset::east_opt(8 * 3600).expect("台北時區");
    let date_time = taipei
        .with_ymd_and_hms(2026, 9, 30, 15, 0, 0)
        .single()
        .expect("有效時間");
    let uri = server.uri();
    let (listed_rows, otc_snapshot) = with_origins(
        &[
            ("https://www.twse.com.tw", &uri),
            ("https://www.tpex.org.tw", &uri),
        ],
        async { tokio::join!(listed(date_time), otc()) },
    )
    .await;
    let saved: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT stock_symbol, issued_share, qfii_shares_held FROM stocks WHERE stock_symbol = ANY($1) ORDER BY stock_symbol",
    )
    .bind(&symbols)
    .fetch_all(pool)
    .await
    .expect("讀回主檔");
    let cached = SHARE.stocks.read().ok().map(|cache| {
        (
            cache.get(LISTED).map(|stock| stock.qfii_shares_held()),
            cache.get(OTC).map(|stock| stock.qfii_shares_held()),
        )
    });
    cleanup().await;

    let listed_rows = listed_rows.expect("上市抓取成功");
    let otc_snapshot = otc_snapshot.expect("上櫃抓取成功");
    assert_eq!(listed_rows.len(), 1);
    assert_eq!(otc_snapshot.date, day(9, 30), "上櫃以櫃買回應的日期為準");
    assert_eq!(
        saved,
        vec![
            (LISTED.to_string(), 1_000, 400),
            (OTC.to_string(), 25_219_056, 22_154_306),
        ]
    );
    assert_eq!(cached, Some((Some(400), Some(22_154_306))), "快取跟著更新");
}

/// 台北時間指定日期 15:00；測試一律用 1991 年的歷史日期，不會碰到真實資料。
fn taipei_local(year: i32, month: u32, day: u32) -> DateTime<Local> {
    let naive = NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|date| date.and_hms_opt(15, 0, 0))
        .expect("有效日期");
    Local
        .from_local_datetime(&naive)
        .single()
        .expect("本地時間沒有歧義")
}

/// 指定資料日的上市 MI_QFIIS 回應（同 [`mi_qfiis`]，日期換掉）。
fn mi_qfiis_on(date: NaiveDate) -> serde_json::Value {
    let mut body = mi_qfiis();
    body["date"] = serde_json::json!(date.format("%Y%m%d").to_string());
    body
}

/// 指定資料日的櫃買 QFII 回應（同 [`tpex_qfii`]，日期換掉）。
fn tpex_qfii_on(date: NaiveDate) -> serde_json::Value {
    let mut body = tpex_qfii();
    body["date"] = serde_json::json!(date.format("%Y%m%d").to_string());
    body
}

async fn mount_sources(server: &MockServer, date: NaiveDate) {
    Mock::given(method("GET"))
        .and(path("/rwd/zh/fund/MI_QFIIS"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mi_qfiis_on(date)))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/www/zh-tw/insti/qfii"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tpex_qfii_on(date)))
        .mount(server)
        .await;
}

/// 種入兩檔假股票，回傳清理時要用的代號。
async fn seed_stocks() -> Vec<String> {
    let pool = crate::infra::database::get_connection();
    let symbols = vec![LISTED.to_string(), OTC.to_string()];
    cleanup_history(&symbols).await;
    for (symbol, market) in [(LISTED, 2), (OTC, 4)] {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, "SuspendListing", issued_share, qfii_shares_held, qfii_share_holding_percentage)
               VALUES ($1, $1, $1, $2, false, 1000, 100, 10)"#,
        )
        .bind(symbol)
        .bind(market)
        .execute(pool)
        .await
        .expect("種入股票主檔");
    }
    symbols
}

/// 清掉假代號的主檔、外資持股歷史與趨勢，以及共享快取。
async fn cleanup_history(symbols: &[String]) {
    let pool = crate::infra::database::get_connection();
    for sql in [
        "DELETE FROM qfii_trend WHERE stock_symbol = ANY($1)",
        "DELETE FROM qfii_history WHERE stock_symbol = ANY($1)",
        "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
    ] {
        sqlx::query(sql)
            .bind(symbols)
            .execute(pool)
            .await
            .unwrap_or_else(|why| panic!("{sql}: {why}"));
    }
    if let Ok(mut cache) = SHARE.stocks.write() {
        cache.remove(LISTED);
        cache.remove(OTC);
    }
}

type SymbolDates = Vec<(String, NaiveDate)>;

async fn history_and_trend(symbols: &[String]) -> (SymbolDates, SymbolDates) {
    let pool = crate::infra::database::get_connection();
    let history = sqlx::query_as(
        r#"SELECT stock_symbol, "date" FROM qfii_history WHERE stock_symbol = ANY($1) ORDER BY stock_symbol, "date""#,
    )
    .bind(symbols)
    .fetch_all(pool)
    .await
    .expect("讀回外資持股歷史");
    let trend = sqlx::query_as(
        r#"SELECT stock_symbol, "date" FROM qfii_trend WHERE stock_symbol = ANY($1) ORDER BY stock_symbol"#,
    )
    .bind(symbols)
    .fetch_all(pool)
    .await
    .expect("讀回外資持股趨勢");
    (history, trend)
}

/// 週末直接結束，不打任何來源。
#[tokio::test]
async fn execute_at_skips_weekend() {
    execute_at(taipei_local(1991, 6, 22))
        .await
        .expect("週末直接結束");
}

/// 平日：上市與上櫃寫回主檔、寫入當天外資持股歷史並重算趨勢；同一天重跑沒有變更時提早結束。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn execute_at_saves_history_and_trend_then_skips_unchanged_rerun() {
    dotenvy::dotenv().ok();
    if crate::infra::database::ping().await.is_err() {
        println!(
            "跳過 execute_at_saves_history_and_trend_then_skips_unchanged_rerun：無資料庫連接"
        );
        return;
    }
    let symbols = seed_stocks().await;
    let date = NaiveDate::from_ymd_opt(1991, 6, 20).expect("有效日期");
    let server = MockServer::start().await;
    mount_sources(&server, date).await;
    let uri = server.uri();
    let origins = [
        ("https://www.twse.com.tw", uri.as_str()),
        ("https://www.tpex.org.tw", uri.as_str()),
    ];

    let first = with_origins(&origins, execute_at(taipei_local(1991, 6, 20))).await;
    let after_first = history_and_trend(&symbols).await;
    let rerun = with_origins(&origins, execute_at(taipei_local(1991, 6, 20))).await;
    let after_rerun = history_and_trend(&symbols).await;
    cleanup_history(&symbols).await;

    first.expect("第一次執行成功");
    rerun.expect("同一天重跑成功（沒有變更）");
    let expected = vec![(LISTED.to_string(), date), (OTC.to_string(), date)];
    assert_eq!(
        after_first.0, expected,
        "上市以查詢日、上櫃以回應日期寫入歷史"
    );
    assert_eq!(after_first.1, expected, "兩檔都重算到當天的趨勢");
    assert_eq!(after_rerun, after_first, "重跑不新增歷史、不改趨勢");
}

/// 歷史回補：只抓區間內的平日，寫入有資料的交易日後以最後一天重算趨勢，不派發通知。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn backfill_history_fetches_weekdays_and_rebuilds_trend() {
    dotenvy::dotenv().ok();
    if crate::infra::database::ping().await.is_err() {
        println!("跳過 backfill_history_fetches_weekdays_and_rebuilds_trend：無資料庫連接");
        return;
    }
    let symbols = seed_stocks().await;
    let friday = NaiveDate::from_ymd_opt(1991, 6, 21).expect("有效日期");
    let sunday = NaiveDate::from_ymd_opt(1991, 6, 23).expect("有效日期");
    let server = MockServer::start().await;
    mount_sources(&server, friday).await;
    let uri = server.uri();

    let summary = with_origins(
        &[
            ("https://www.twse.com.tw", uri.as_str()),
            ("https://www.tpex.org.tw", uri.as_str()),
        ],
        backfill_history(friday, sunday),
    )
    .await;
    let (history, trend) = history_and_trend(&symbols).await;
    let requests = server
        .received_requests()
        .await
        .map_or(0, |requests| requests.len());
    cleanup_history(&symbols).await;

    let summary = summary.expect("回補成功");
    assert_eq!(
        (summary.trading_days, summary.saved_rows),
        (1, 2),
        "週六、週日不抓，週五兩檔寫入"
    );
    assert!(summary.trend_rows >= 2, "以週五重算趨勢：{summary:?}");
    assert_eq!(requests, 2, "只有週五打上市與上櫃各一次");
    let expected = vec![(LISTED.to_string(), friday), (OTC.to_string(), friday)];
    assert_eq!((history, trend), (expected.clone(), expected));
}
