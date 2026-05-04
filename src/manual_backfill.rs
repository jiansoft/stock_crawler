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
//! - `test_backfill_received_dividend_records_for_stock`：
//!   依 [`MANUAL_DIVIDEND_RECORD_SECURITY_CODE`] 重算指定股票目前持股的已領股利總表與明細。
//! - `test_backfill_historical_dividends_for_stock`：
//!   依 [`MANUAL_HISTORICAL_DIVIDEND_SECURITY_CODE`] 從 Yahoo 回補單檔股票歷年股利，
//!   寫入 `dividend` 表、重算年度彙總列，並同步回補已領股利紀錄。

use chrono::NaiveDate;

use crate::{
    backfill::{dividend, quote},
    cache::SHARE,
    calculation::dividend_record,
    database,
    event::taiwan_stock::closing,
    logging,
};

/// 手動回補各股每日收盤報價時使用的預設交易日。
const MANUAL_DAILY_QUOTE_DATE: &str = "2026-04-30";

/// 手動回補收盤事件匯總時使用的預設交易日。
const MANUAL_CLOSING_AGGREGATE_DATE: &str = "2026-04-30";

/// 手動回補已領股利紀錄時使用的預設股票代號。
const MANUAL_DIVIDEND_RECORD_SECURITY_CODE: &str = "0056";

/// 手動回補單檔歷年股利時使用的預設股票代號。
const MANUAL_HISTORICAL_DIVIDEND_SECURITY_CODE: &str = "2887";

/// 手動回補指定交易日的各股每日收盤報價。
///
/// 此測試等同把原本的 `backfill::quote::tests::test_execute` 集中到手動回補檔。
/// 它會先刪除指定交易日既有的 `DailyQuotes`，再重新呼叫 TWSE 與 TPEx 來源抓取
/// 上市櫃各股開高低收、成交量與本益比等欄位，最後批次寫回資料庫並更新快取。
///
/// 執行範例：
/// `cargo test manual_backfill::test_backfill_daily_quotes_for_date -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_daily_quotes_for_date() {
    dotenv::dotenv().ok();
    SHARE.load().await;

    let date = NaiveDate::parse_from_str(MANUAL_DAILY_QUOTE_DATE, "%Y-%m-%d")
        .expect("manual daily quote date should be valid");

    logging::debug_file_async(format!(
        "開始 manual_backfill::test_backfill_daily_quotes_for_date date={date}"
    ));

    // quote::execute 使用 COPY 寫入 DailyQuotes；先清掉同日資料可避免唯一索引衝突，
    // 也讓這個手動回補確實以外部來源的最新內容重建當日各股收盤報價。
    sqlx::query(r#"delete from "DailyQuotes" where "Date" = $1;"#)
        .bind(date)
        .execute(database::get_connection())
        .await
        .expect("delete existing manual daily quotes failed");

    let quote_count = quote::execute(date)
        .await
        .expect("manual daily quote backfill failed");

    logging::debug_file_async(format!(
        "結束 manual_backfill::test_backfill_daily_quotes_for_date date={date} quote_count={quote_count}"
    ));
}

/// 手動執行每日收盤事件主要匯總流程。
///
/// 此測試等同把原本的 `event::taiwan_stock::closing::tests::test_aggregate`
/// 集中到手動回補檔。它會依指定交易日重跑收盤報價回補、缺漏補齊、均線、
/// last daily quote、估價、殖利率排行、市值重算與通知前置資料。
///
/// 執行範例：
/// `cargo test manual_backfill::test_backfill_closing_aggregate_for_date -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_closing_aggregate_for_date() {
    dotenv::dotenv().ok();
    SHARE.load().await;

    let date = NaiveDate::parse_from_str(MANUAL_CLOSING_AGGREGATE_DATE, "%Y-%m-%d")
        .expect("manual closing aggregate date should be valid");

    logging::debug_file_async(format!(
        "開始 manual_backfill::test_backfill_closing_aggregate_for_date date={date}"
    ));

    closing::aggregate(date)
        .await
        .expect("manual closing aggregate backfill failed");

    logging::debug_file_async(format!(
        "結束 manual_backfill::test_backfill_closing_aggregate_for_date date={date}"
    ));
}

/// 手動回補指定股票目前持股的已領股利紀錄。
///
/// 此測試等同把原本的
/// `calculation::dividend_record::tests::test_backfill_received_dividend_records_for_stock_backfills_after_dividend_insert`
/// 集中到手動回補檔。它會依股票代號找出目前持股與既有股利年度，
/// 並重算 `dividend_record_detail` 與 `dividend_record_detail_more`。
///
/// 執行範例：
/// `cargo test manual_backfill::test_backfill_received_dividend_records_for_stock -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_received_dividend_records_for_stock() {
    dotenv::dotenv().ok();
    SHARE.load().await;

    let security_code = MANUAL_DIVIDEND_RECORD_SECURITY_CODE;
    logging::debug_file_async(format!(
        "開始 manual_backfill::test_backfill_received_dividend_records_for_stock security_code={security_code}"
    ));

    let summary = dividend_record::backfill_received_dividend_records_for_stock(security_code)
        .await
        .expect("manual received dividend records backfill failed");

    logging::debug_file_async(format!(
        "結束 manual_backfill::test_backfill_received_dividend_records_for_stock security_code={security_code} summary={summary:?}"
    ));
}

/// 手動回補指定股票在 Yahoo 可取得的歷年股利明細。
///
/// 此測試會呼叫股利回補子流程 [`dividend::backfill_historical_dividends_for_stock`]，
/// 將單檔股票的歷年股利資料 upsert 回 `dividend` 表；若來源含季配或半年配，
/// 也會重算年度彙總列，最後同步回補目前持股的已領股利紀錄。
///
/// 執行範例：
/// `cargo test manual_backfill::test_backfill_historical_dividends_for_stock -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_historical_dividends_for_stock() {
    dotenv::dotenv().ok();
    SHARE.load().await;

    let security_code = MANUAL_HISTORICAL_DIVIDEND_SECURITY_CODE;
    logging::debug_file_async(format!(
        "開始 manual_backfill::test_backfill_historical_dividends_for_stock security_code={security_code}"
    ));

    let upserted_count = dividend::backfill_historical_dividends_for_stock(security_code)
        .await
        .expect("manual historical dividends backfill failed");

    logging::debug_file_async(format!(
        "結束 manual_backfill::test_backfill_historical_dividends_for_stock security_code={security_code} upserted_count={upserted_count}"
    ));
}
