//! 手動資料回補測試入口。
//!
//! 這個檔案集中放置平常不應自動執行、但缺資料時可用 `cargo test ... -- --ignored`
//! 直接觸發的回補測試。每個測試都依賴本機 `.env`、資料庫與外部資料來源，
//! 因此一律標記為 `#[ignore]`。
//!
//! 目前提供下列手動回補操作：
//!
//! - `test_backfill_daily_quotes_for_date`：
//!   依 [`MANUAL_DAILY_QUOTE_DATE`] 重新抓取上市櫃各股每日收盤報價，寫入 `DailyQuotes`。
//! - `test_backfill_closing_aggregate_for_date`：
//!   依 [`MANUAL_CLOSING_AGGREGATE_DATE`] 重跑每日收盤事件匯總，包含收盤報價回補、
//!   缺漏補齊、均線、最後交易日報價、估價、殖利率排行與市值重算。
//! - `test_backfill_taiwan_stock_index`：
//!   依 [`MANUAL_TAIWAN_STOCK_INDEX_DATE`] 回補指定日期的台股加權指數，
//!   跳過快取檢查後 upsert 寫入 `Index` 並更新快取。
//! - `test_backfill_received_dividend_records_for_stock`：
//!   依 [`MANUAL_DIVIDEND_RECORD_SECURITY_CODE`] 重算指定股票目前持股的已領股利總表與明細。
//! - `test_backfill_historical_dividends_for_stock`：
//!   依 [`MANUAL_HISTORICAL_DIVIDEND_SECURITY_CODE`] 從 Yahoo 回補單檔股票歷年股利，
//!   寫入 `dividend` 表、重算年度彙總列，並同步回補已領股利紀錄。
//! - `test_backfill_cagr_for_date`：
//!   依 [`MANUAL_CAGR_DATE`] 重算指定基準日的全市場各期間年化報酬率，寫入 `stock_cagr`。

use chrono::NaiveDate;

use crate::{
    app::backfill::{dividend, quote, taiwan_stock_index},
    app::calculation::{cagr, dividend_record},
    app::event::taiwan_stock::closing,
    infra::cache::SHARE,
};

/// 手動回補各股每日收盤報價時使用的預設交易日。
const MANUAL_DAILY_QUOTE_DATE: &str = "2026-04-30";

/// 手動回補收盤事件匯總時使用的預設交易日。
const MANUAL_CLOSING_AGGREGATE_DATE: &str = "2026-04-30";

/// 手動回補已領股利紀錄時使用的預設股票代號。
const MANUAL_DIVIDEND_RECORD_SECURITY_CODE: &str = "0056";

/// 手動回補單檔歷年股利時使用的預設股票代號。
const MANUAL_HISTORICAL_DIVIDEND_SECURITY_CODE: &str = "2887";

/// 手動重算各期間年化報酬率時使用的預設基準日。
///
/// 空字串表示改用資料庫中最新的交易日。
const MANUAL_CAGR_DATE: &str = "";

/// 手動回補指定交易日的各股每日收盤報價。
///
/// 此測試等同把原本的 `backfill::quote::tests::test_execute` 集中到手動回補檔。
/// 它會重新呼叫 TWSE 與 TPEx 來源抓取上市櫃各股開高低收、成交量與本益比等欄位，
/// 抓取成功後才在單一 transaction 內刪除同日舊資料並批次寫回資料庫、更新快取。
///
/// 執行範例：
/// `cargo test app::manual_backfill::test_backfill_daily_quotes_for_date -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_daily_quotes_for_date() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let date = NaiveDate::parse_from_str(MANUAL_DAILY_QUOTE_DATE, "%Y-%m-%d")
        .expect("manual daily quote date should be valid");

    tracing::debug!("開始 app::manual_backfill::test_backfill_daily_quotes_for_date date={date}");

    // quote::execute 內部採「先抓取、後原子替換」：同日舊資料的刪除與新資料的
    // COPY 寫入綁在同一個 transaction，抓取或寫入失敗都不會留下資料缺口，
    // 因此這裡不需要先手動刪除當日資料。
    let quote_count = quote::execute(date)
        .await
        .expect("manual daily quote backfill failed");

    tracing::debug!(
        "結束 app::manual_backfill::test_backfill_daily_quotes_for_date date={date} quote_count={quote_count}"
    );
}

/// 手動執行每日收盤事件主要匯總流程。
///
/// 此測試等同把原本的 `event::taiwan_stock::closing::tests::test_aggregate`
/// 集中到手動回補檔。它會依指定交易日重跑收盤報價回補、缺漏補齊、均線、
/// last daily quote、估價、殖利率排行、市值重算與通知前置資料。
///
/// 執行範例：
/// `cargo test app::manual_backfill::test_backfill_closing_aggregate_for_date -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_closing_aggregate_for_date() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let date = NaiveDate::parse_from_str(MANUAL_CLOSING_AGGREGATE_DATE, "%Y-%m-%d")
        .expect("manual closing aggregate date should be valid");

    tracing::debug!(
        "開始 app::manual_backfill::test_backfill_closing_aggregate_for_date date={date}"
    );

    closing::aggregate(date)
        .await
        .expect("manual closing aggregate backfill failed");

    tracing::debug!(
        "結束 app::manual_backfill::test_backfill_closing_aggregate_for_date date={date}"
    );
}

/// 手動回補台股加權指數時使用的預設日期。
///
/// TWSE API 會依此日期回傳該月份所有交易日的指數資料。
const MANUAL_TAIWAN_STOCK_INDEX_DATE: &str = "2026-04-15";

/// 手動回補指定月份的台股加權指數。
///
/// 此測試等同把原本的 `backfill::taiwan_stock_index::tests::test_execute`
/// 集中到手動回補檔。它會使用 [`MANUAL_TAIWAN_STOCK_INDEX_DATE`] 呼叫 TWSE
/// 加權股價指數來源，將該月份所有交易日的指數 upsert 回 `Index`，並更新記憶體快取。
/// 回補模式會跳過快取檢查，確保所有資料都寫入資料庫。
///
/// 執行範例：
/// `cargo test app::manual_backfill::test_backfill_taiwan_stock_index -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_taiwan_stock_index() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let date = NaiveDate::parse_from_str(MANUAL_TAIWAN_STOCK_INDEX_DATE, "%Y-%m-%d")
        .expect("manual taiwan stock index date should be valid");

    tracing::debug!("開始 app::manual_backfill::test_backfill_taiwan_stock_index date={date}");

    let upserted_count = taiwan_stock_index::execute_for_date(date)
        .await
        .expect("manual taiwan stock index backfill failed");

    tracing::debug!(
        "結束 app::manual_backfill::test_backfill_taiwan_stock_index date={date} upserted_count={upserted_count}"
    );
}

/// 手動回補指定股票目前持股的已領股利紀錄。
///
/// 此測試等同把原本的
/// `calculation::dividend_record::tests::test_backfill_received_dividend_records_for_stock_backfills_after_dividend_insert`
/// 集中到手動回補檔。它會依股票代號找出目前持股與既有股利年度，
/// 並重算 `dividend_record_detail` 與 `dividend_record_detail_more`。
///
/// 執行範例：
/// `cargo test app::manual_backfill::test_backfill_received_dividend_records_for_stock -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_received_dividend_records_for_stock() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let security_code = MANUAL_DIVIDEND_RECORD_SECURITY_CODE;
    tracing::debug!(
        "開始 app::manual_backfill::test_backfill_received_dividend_records_for_stock security_code={security_code}"
    );

    let summary = dividend_record::backfill_received_dividend_records_for_stock(security_code)
        .await
        .expect("manual received dividend records backfill failed");

    tracing::debug!(
        "結束 app::manual_backfill::test_backfill_received_dividend_records_for_stock security_code={security_code} summary={summary:?}"
    );
}

/// 手動回補指定股票在 Yahoo 可取得的歷年股利明細。
///
/// 此測試會呼叫股利回補子流程 [`dividend::backfill_historical_dividends_for_stock`]，
/// 將單檔股票的歷年股利資料 upsert 回 `dividend` 表；若來源含季配或半年配，
/// 也會重算年度彙總列，最後同步回補目前持股的已領股利紀錄。
///
/// 執行範例：
/// `cargo test app::manual_backfill::test_backfill_historical_dividends_for_stock -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_historical_dividends_for_stock() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let security_code = MANUAL_HISTORICAL_DIVIDEND_SECURITY_CODE;
    tracing::debug!(
        "開始 app::manual_backfill::test_backfill_historical_dividends_for_stock security_code={security_code}"
    );

    let upserted_count = dividend::backfill_historical_dividends_for_stock(security_code)
        .await
        .expect("manual historical dividends backfill failed");

    tracing::debug!(
        "結束 app::manual_backfill::test_backfill_historical_dividends_for_stock security_code={security_code} upserted_count={upserted_count}"
    );
}

/// 手動重算指定基準日的全市場各期間年化報酬率（CAGR）。
///
/// 排程本身每日 05:40（台北時間）自動執行，這個入口用於：
/// 股利資料事後回補後需要重算、或首次上線時補算某一天的結果。
///
/// 計算完全依賴資料庫既有的報價與股利，不會呼叫任何外部網站；
/// 同一 `(基準日, 股票, 期間)` 重複執行為冪等的 upsert 覆蓋。
///
/// 執行範例：
/// `cargo test app::manual_backfill::test_backfill_cagr_for_date -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_cagr_for_date() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    // 空字串代表交由 use case 自行採用資料庫中最新的交易日。
    let date = if MANUAL_CAGR_DATE.is_empty() {
        None
    } else {
        Some(
            NaiveDate::parse_from_str(MANUAL_CAGR_DATE, "%Y-%m-%d")
                .expect("manual cagr date should be valid"),
        )
    };

    tracing::debug!("開始 app::manual_backfill::test_backfill_cagr_for_date date={date:?}");

    let summary = cagr::execute(date)
        .await
        .expect("manual cagr backfill failed");

    tracing::debug!(
        "結束 app::manual_backfill::test_backfill_cagr_for_date date={:?} universe={} periods_calculated={} periods_skipped={} rows_written={} anomaly_symbols={}",
        summary.date,
        summary.universe,
        summary.periods_calculated,
        summary.periods_skipped,
        summary.rows_written,
        summary.anomaly_symbols
    );
}
