//! 手動回補：股利、已領股利與除權息核對。
//!
//! - `test_backfill_received_dividend_records_for_stock`：
//!   依 [`MANUAL_DIVIDEND_RECORD_SECURITY_CODE`] 重算指定股票目前持股的已領股利總表與明細。
//! - `test_backfill_historical_dividends_for_stock`：
//!   依 [`MANUAL_HISTORICAL_DIVIDEND_SECURITY_CODE`] 從 Yahoo 回補單檔股票歷年股利，
//!   寫入 `dividend` 表、重算年度彙總列，並同步回補已領股利紀錄。
//! - `test_backfill_historical_dividends_for_multiple_dividend_stocks`：
//!   對指定年度（`MANUAL_HISTORICAL_DIVIDEND_YEAR`，預設今年）所有季配/半年配股票
//!   逐檔執行上一項的歷年股利回補。
//! - `test_repair_stale_annual_total_dividends`：
//!   手動修復帶有配息日期的年度合計列。
//! - `test_reconcile_ex_right_results`：
//!   以交易所除權除息計算結果核對 `MANUAL_EX_RIGHT_FROM`～`MANUAL_EX_RIGHT_TO` 的股利日期與金額；
//!   預設只試算，`MANUAL_EX_RIGHT_APPLY=1` 才寫入。

use chrono::NaiveDate;

use crate::app::backfill::dividend;
use crate::app::calculation::dividend_record;
use crate::infra::cache::SHARE;

/// 手動回補已領股利紀錄時使用的預設股票代號。
const MANUAL_DIVIDEND_RECORD_SECURITY_CODE: &str = "0056";

/// 手動回補單檔歷年股利時使用的預設股票代號。
const MANUAL_HISTORICAL_DIVIDEND_SECURITY_CODE: &str = "2887";

/// 手動回補指定股票目前持股的已領股利紀錄。
///
/// 此測試等同把原本的
/// `calculation::dividend_record::tests::test_backfill_received_dividend_records_for_stock_backfills_after_dividend_insert`
/// 集中到手動回補檔。它會依股票代號找出目前持股與既有股利年度，
/// 並重算 `dividend_record_detail` 與 `dividend_record_detail_more`。
///
/// 執行範例：
/// `cargo test app::manual_backfill::dividend::test_backfill_received_dividend_records_for_stock -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_received_dividend_records_for_stock() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let security_code = MANUAL_DIVIDEND_RECORD_SECURITY_CODE;
    tracing::debug!(
        "開始 app::manual_backfill::dividend::test_backfill_received_dividend_records_for_stock security_code={security_code}"
    );

    let summary = dividend_record::backfill_received_dividend_records_for_stock(security_code)
        .await
        .expect("manual received dividend records backfill failed");

    tracing::debug!(
        "結束 app::manual_backfill::dividend::test_backfill_received_dividend_records_for_stock security_code={security_code} summary={summary:?}"
    );
}

/// 手動回補指定股票在 Yahoo 可取得的歷年股利明細。
///
/// 此測試會呼叫股利回補子流程 [`dividend::backfill_historical_dividends_for_stock`]，
/// 將單檔股票的歷年股利資料 upsert 回 `dividend` 表；若來源含季配或半年配，
/// 也會重算年度彙總列，最後同步回補目前持股的已領股利紀錄。
///
/// 執行範例：
/// `cargo test app::manual_backfill::dividend::test_backfill_historical_dividends_for_stock -- --ignored --nocapture`
/// 可用 `MANUAL_DIVIDEND_SECURITY_CODE` 指定股票；未設定時沿用預設代號。
#[tokio::test]
#[ignore]
async fn test_backfill_historical_dividends_for_stock() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let security_code = std::env::var("MANUAL_DIVIDEND_SECURITY_CODE")
        .unwrap_or_else(|_| MANUAL_HISTORICAL_DIVIDEND_SECURITY_CODE.to_string());
    tracing::debug!(
        "開始 app::manual_backfill::dividend::test_backfill_historical_dividends_for_stock security_code={security_code}"
    );

    let upserted_count = dividend::backfill_historical_dividends_for_stock(&security_code)
        .await
        .expect("manual historical dividends backfill failed");

    tracing::debug!(
        "結束 app::manual_backfill::dividend::test_backfill_historical_dividends_for_stock security_code={security_code} upserted_count={upserted_count}"
    );
}

/// 手動回補指定年度所有季配/半年配股票的 Yahoo 歷年股利明細。
///
/// 呼叫 [`dividend::backfill_historical_dividends_for_multiple_dividend_stocks`]：
/// 先從資料庫找出該年度有季配/半年配紀錄的股票，再逐檔從 Yahoo 回補歷年股利；
/// 任一檔失敗即中止，每檔之間隨機停 1.5～3 秒。
///
/// 執行範例：
/// `cargo test app::manual_backfill::dividend::test_backfill_historical_dividends_for_multiple_dividend_stocks -- --ignored --nocapture`
/// 可用 `MANUAL_HISTORICAL_DIVIDEND_YEAR` 指定年度；未設定時為今年。
#[tokio::test]
#[ignore]
async fn test_backfill_historical_dividends_for_multiple_dividend_stocks() {
    use chrono::Datelike;

    dotenvy::dotenv().ok();
    SHARE.load().await;

    let year = std::env::var("MANUAL_HISTORICAL_DIVIDEND_YEAR")
        .ok()
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or_else(|| chrono::Local::now().year());

    let summary = dividend::backfill_historical_dividends_for_multiple_dividend_stocks(year)
        .await
        .unwrap_or_else(|why| {
            panic!(
                "backfill historical dividends failed for all multiple dividend stocks in {year}: {why:#}"
            )
        });

    assert!(
        summary.stock_count > 0,
        "expected at least one multiple dividend stock for {year}"
    );
    assert!(
        summary.detail_count > 0,
        "expected Yahoo historical dividends to upsert at least one row for multiple dividend stocks in {year}"
    );
}

/// 手動修復帶有配息日期的年度合計列。
///
/// 年度合計列 (`quarter = ''`) 的日期應該一律是 `'-'`；股票從年配改成分期配發時，
/// 原本的年度配息明細會與合計列撞上同一組主鍵，舊版只覆寫金額而留下日期，
/// 讓合計被當成另一次真實配息重複計入持股的已領股利。
///
/// 此入口掃出所有受影響的 (代號, 發放年度)，重算合計列把日期清回 `'-'`，
/// 再重算對應股票目前持股的已領股利紀錄。全程只讀寫資料庫，不會請求 Yahoo。
///
/// 執行範例：
/// `cargo test app::manual_backfill::dividend::test_repair_stale_annual_total_dividends -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_repair_stale_annual_total_dividends() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    tracing::debug!(
        "開始 app::manual_backfill::dividend::test_repair_stale_annual_total_dividends"
    );

    let summary = dividend::repair_stale_annual_total_dividends()
        .await
        .expect("manual stale annual total dividend repair failed");

    tracing::debug!(
        "結束 app::manual_backfill::dividend::test_repair_stale_annual_total_dividends summary={summary:?}"
    );
}

/// 以交易所除權除息計算結果核對指定期間的股利日期與金額。
///
/// 預設只試算並印出摘要；設 `MANUAL_EX_RIGHT_APPLY=1` 才寫入（含從 Yahoo 補缺漏，每檔間隔 2 秒）。
/// 排程每天只核對近 45 天，
/// 歷史資料用這個入口一次核對（交易所資料自 2010 年前後起可查）。
///
/// 執行範例：
/// `MANUAL_EX_RIGHT_FROM=2019-01-01 MANUAL_EX_RIGHT_TO=2026-10-01 cargo test app::manual_backfill::dividend::test_reconcile_ex_right_results -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_reconcile_ex_right_results() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let parse = |name: &str, default: &str| {
        let value = std::env::var(name).unwrap_or_else(|_| default.to_string());
        NaiveDate::parse_from_str(&value, "%Y-%m-%d")
            .unwrap_or_else(|_| panic!("{name} should be YYYY-MM-DD, got {value}"))
    };
    let from = parse("MANUAL_EX_RIGHT_FROM", "2019-01-01");
    let to = parse("MANUAL_EX_RIGHT_TO", "2026-10-01");
    let apply = std::env::var("MANUAL_EX_RIGHT_APPLY").is_ok_and(|value| value == "1");

    let summary = dividend::ex_right_reconcile::execute(from, to, apply)
        .await
        .expect("ex-right reconciliation failed");

    println!("結束 test_reconcile_ex_right_results {from}~{to} apply={apply} {summary:?}");
}
