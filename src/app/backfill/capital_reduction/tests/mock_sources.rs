//! 以本機 mock server 取代 TWSE 與 TPEx（G15）：減資、變更面額、ETF 分割與上櫃減資的回補入口。

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::core::util::http::mock_origin::with_origins;

const TWSE: &str = "https://www.twse.com.tw";
const TPEX: &str = "https://www.tpex.org.tw";
const LISTED_REDUCTION: &str = "/rwd/zh/reducation/TWTAUU";
const PAR_VALUE_CHANGE: &str = "/rwd/zh/change/TWTB8U";
const ETF_SPLIT: &str = "/rwd/zh/split/TWTCAU";
const OTC_REDUCTION: &str = "/www/zh-tw/bulletin/revivt";
/// 上市減資、變更面額、ETF 反分割、上櫃減資各一檔假代號。
const SYMBOLS: [&str; 4] = ["79979KA", "79979KB", "79979KC", "79979KD"];

/// TWTAUU：上市減資一筆（停止買賣前 6.58、恢復買賣參考價 13.33）。
fn listed_reduction() -> serde_json::Value {
    serde_json::json!({
        "stat": "OK",
        "fields": ["恢復買賣日期", "股票代號", "名稱", "停止買賣前收盤價格", "恢復買賣參考價",
                   "漲停價格", "跌停價格", "開盤競價基準", "除權參考價", "減資原因", "詳細資料"],
        "data": [["104/03/20", SYMBOLS[0], "測試減資", "6.58", "13.33", "14.25", "12.15",
                  "13.35", "--", "彌補虧損", "79979KA,20150107"]]
    })
}

/// TWTB8U：變更面額一筆（190 → 19，1 股拆 10 股）。
fn par_value_change() -> serde_json::Value {
    serde_json::json!({
        "stat": "OK",
        "fields": ["恢復買賣日期", "股票代號", "名稱", "停止買賣前收盤價格", "恢復買賣參考價",
                   "漲停價格", "跌停價格", "開盤競價基準", "詳細資料"],
        "data": [["109/08/17", SYMBOLS[1], "測試面額", "190.00", "19.00", "20.90", "17.10",
                  "19.00", "79979KB,20200806,20200817"]]
    })
}

/// TWTCAU：ETF 反分割一筆（3.28 → 22.96，7 股併 1 股）。
fn etf_split() -> serde_json::Value {
    serde_json::json!({
        "stat": "OK",
        "fields": ["恢復買賣日期", "ETF代號", "名稱", "分割(反分割)", "停止買賣前收盤價格",
                   "恢復買賣參考價", "漲停價格", "跌停價格", "開盤競價基準"],
        "data": [["113/12/11", SYMBOLS[2], "測試反分割", "反分割", "3.28", "22.96", "25.25",
                  "20.67", "22.96"]]
    })
}

/// revivt：上櫃減資一筆（5.13 → 8.32）。
fn otc_reduction() -> serde_json::Value {
    serde_json::json!({
        "stat": "ok",
        "tables": [{
            "fields": ["恢復買賣日期", "股票代號", "名稱", "最後交易日之收盤價格",
                       "減資恢復買賣開始日參考價格", "漲停價格", "跌停價格", "開始交易基準價",
                       "除權參考價", "減資原因", "詳細資料"],
            "data": [["1150921", SYMBOLS[3], "測試上櫃減資", "5.13", "8.32", "9.15", "7.49",
                      "8.32", "0.00", "彌補虧損", ""]]
        }]
    })
}

/// 四個來源都掛上；`failing` 列出的路徑改回 HTTP 500。
async fn sources(failing: &[&str]) -> MockServer {
    let server = MockServer::start().await;
    for (route, body) in [
        (LISTED_REDUCTION, listed_reduction()),
        (PAR_VALUE_CHANGE, par_value_change()),
        (ETF_SPLIT, etf_split()),
        (OTC_REDUCTION, otc_reduction()),
    ] {
        let response = if failing.contains(&route) {
            ResponseTemplate::new(500)
        } else {
            ResponseTemplate::new(200).set_body_json(body)
        };
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(response)
            .mount(&server)
            .await;
    }
    server
}

async fn within<T>(server: &MockServer, fut: impl Future<Output = T>) -> T {
    let uri = server.uri();
    with_origins(&[(TWSE, &uri), (TPEX, &uri)], fut).await
}

async fn cleanup() {
    sqlx::query("DELETE FROM corporate_action WHERE stock_symbol = ANY($1)")
        .bind(SYMBOLS)
        .execute(crate::infra::database::get_connection())
        .await
        .expect("清除測試公司行動");
}

/// 讀回測試代號的 (代號, 類型)，依代號排序。
async fn saved() -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT stock_symbol, action_type FROM corporate_action
         WHERE stock_symbol = ANY($1) ORDER BY stock_symbol",
    )
    .bind(SYMBOLS)
    .fetch_all(crate::infra::database::get_connection())
    .await
    .expect("讀回測試公司行動")
}

async fn database_unavailable(test: &str) -> bool {
    dotenvy::dotenv().ok();
    if crate::infra::database::ping().await.is_err() {
        println!("跳過 {test}：無資料庫連接");
        return true;
    }
    false
}

/// 每日排程：上市減資、變更面額、ETF 分割與上櫃減資都寫入。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn execute_saves_every_source() {
    if database_unavailable("execute_saves_every_source").await {
        return;
    }
    cleanup().await;
    let server = sources(&[]).await;

    let result = within(&server, execute()).await;
    let rows = saved().await;
    cleanup().await;

    result.expect("減資回補完成");
    let types: Vec<&str> = rows.iter().map(|(_, kind)| kind.as_str()).collect();
    assert_eq!(
        rows.iter()
            .map(|(symbol, _)| symbol.as_str())
            .collect::<Vec<_>>(),
        SYMBOLS,
        "四個來源各寫入一筆：{rows:?}"
    );
    assert_eq!(types[0], "capital_reduction");
    assert_eq!(types[3], "capital_reduction");
}

/// 上市減資失敗不影響分割與上櫃；分割兩張表任一失敗就整批不寫。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn execute_isolates_failed_sources() {
    if database_unavailable("execute_isolates_failed_sources").await {
        return;
    }
    cleanup().await;
    let server = sources(&[LISTED_REDUCTION, ETF_SPLIT]).await;

    let result = within(&server, execute()).await;
    let rows = saved().await;
    cleanup().await;

    result.expect("單一來源失敗只記錄，不回傳錯誤");
    assert_eq!(
        rows,
        vec![(SYMBOLS[3].to_owned(), "capital_reduction".to_owned())],
        "上市減資失敗、ETF 分割失敗連帶變更面額也不寫，只剩上櫃"
    );
}

/// 手動全量回補：上市減資加上兩種分割，回傳寫入筆數；任一來源失敗回錯誤。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn backfill_listed_full_counts_reductions_and_splits() {
    if database_unavailable("backfill_listed_full_counts_reductions_and_splits").await {
        return;
    }
    cleanup().await;
    let server = sources(&[]).await;
    let written = within(&server, backfill_listed_full()).await;
    let rows = saved().await;
    cleanup().await;

    assert_eq!(written.expect("全量回補完成"), 3);
    assert_eq!(rows.len(), 3, "上櫃不在全量回補範圍：{rows:?}");

    let failing = sources(&[PAR_VALUE_CHANGE]).await;
    let err = within(&failing, backfill_listed_full())
        .await
        .expect_err("分割來源失敗要回錯誤");
    cleanup().await;
    assert!(format!("{err:#}").contains("par value"), "{err:#}");
}
