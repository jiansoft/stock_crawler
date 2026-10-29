use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use chrono::{Local, NaiveDate};

use crate::{
    app::backfill::{dividend, quote, quote_history, taiwan_stock_index},
    app::calculation::{cagr, daily_quotes, dividend_record},
    app::event::taiwan_stock::closing,
    domain::performance::CagrPeriod,
};

use super::state::{
    BACKFILL_STATE, BackfillJob, BackfillJobStatus, BackfillWebState, MAX_CONCURRENT_JOBS,
};

mod runner;

// 共用的建立流程、逾時與進度回報搬到子模組；各種回補 job 的入口留在這裡。
pub(crate) use self::runner::StartJobError;
use self::runner::*;

/// 建立各股每日收盤報價背景 job。
///
/// Job 會呼叫 TWSE 與 TPEx 來源重抓上市櫃各股每日開高低收、成交量與本益比等
/// 資料，抓取成功後才在單一資料庫 transaction 內刪除同日舊資料並寫入新資料。
pub(crate) async fn start_daily_quotes_job(date: NaiveDate) -> Result<BackfillJob, StartJobError> {
    start_job(
        BACKFILL_STATE.clone(),
        "daily_quotes",
        date.to_string(),
        move || async move {
            // quote::execute 內部採「先抓取、後原子替換」：抓取失敗時完全不動資料庫；
            // 寫入階段的「刪除舊資料 + COPY 新資料」綁在同一個 transaction，
            // 任一步失敗都會 rollback。因此這裡不需要（也不可以）先手動刪除
            // 當日資料——那正是舊版「刪除後抓取失敗，行情永久遺失」的缺陷來源。
            let quote_count = quote::execute(date).await?;
            report_progress(50, "行情已寫入，重新計算均線").await;
            // 替換過去某天的行情會改變之後最多 240 個交易日的均線，從當天起一併重算。
            let recalculated =
                daily_quotes::recalculate_moving_averages_since(date, None).await?;
            report_progress(90, "均線計算完成，整理結果").await;
            Ok(format!(
                "daily quotes backfill completed: quote_count={quote_count}, recalculated={recalculated}"
            ))
        },
    )
    .await
}

/// 建立多次配息股票歷年股利批次回補背景 job。
///
/// Job 會找出指定年度已有季配/半年配資料的股票，逐檔重新抓取 Yahoo 歷年股利，
/// 並把成功處理的股票數與明細 upsert 筆數寫入 job message。
pub(crate) async fn start_multiple_dividend_historical_dividends_job(
    year: i32,
) -> Result<BackfillJob, StartJobError> {
    start_job(
        BACKFILL_STATE.clone(),
        "multiple_dividend_historical_dividends",
        year.to_string(),
        move || async move {
            let summary =
                dividend::backfill_historical_dividends_for_multiple_dividend_stocks(year).await?;
            Ok(format!(
                "multiple dividend historical dividends backfill completed: stock_count={}, detail_count={}",
                summary.stock_count, summary.detail_count
            ))
        },
    )
    .await
}

/// 建立年度合計列修復背景 job。
///
/// Job 會掃出所有帶有配息日期的年度合計列（`quarter = ''`），重算合計把日期清回 `'-'`，
/// 再重算對應股票目前持股的已領股利紀錄。全程只讀寫資料庫，不對外部來源發出請求，
/// 因此可以安全重跑；沒有受影響的資料時 job 立即完成。
pub(crate) async fn start_annual_total_repair_job() -> Result<BackfillJob, StartJobError> {
    start_job(
        BACKFILL_STATE.clone(),
        "annual_total_repair",
        // 這個 job 沒有輸入參數，固定值讓同一時間只會有一個修復流程在跑。
        "all".to_string(),
        move || async move {
            let summary = dividend::repair_stale_annual_total_dividends().await?;
            Ok(format!(
                "annual total repair completed: stale_count={}, repaired_count={}, record_backfilled_count={}",
                summary.stale_count, summary.repaired_count, summary.record_backfilled_count
            ))
        },
    )
    .await
}

/// 建立收盤彙總背景 job。
///
/// Job 會呼叫 `closing::aggregate` 重新彙總指定交易日的收盤資料。
pub(crate) async fn start_closing_aggregate_job(
    date: NaiveDate,
) -> Result<BackfillJob, StartJobError> {
    start_job(
        BACKFILL_STATE.clone(),
        "closing_aggregate",
        date.to_string(),
        move || async move {
            closing::aggregate(date).await?;
            Ok("closing aggregate backfill completed".to_string())
        },
    )
    .await
}

/// 建立台股加權指數背景 job。
///
/// Job 會依指定日期呼叫 TWSE 加權股價指數來源，從回傳的整月資料中篩選出
/// 指定日期的那一筆，跳過快取檢查後 upsert `Index` 資料並更新快取。
pub(crate) async fn start_taiwan_stock_index_job(
    date: NaiveDate,
) -> Result<BackfillJob, StartJobError> {
    start_job(
        BACKFILL_STATE.clone(),
        "taiwan_stock_index",
        date.to_string(),
        move || async move {
            let upserted_count = taiwan_stock_index::execute_for_date(date).await?;
            Ok(format!(
                "taiwan stock index backfill completed: upserted_count={upserted_count}"
            ))
        },
    )
    .await
}

/// 建立持股已領股利重算背景 job。
///
/// Job 會針對單一證券代號呼叫已領股利紀錄回補流程，完成後將摘要寫入 message。
pub(crate) async fn start_received_dividend_records_job(
    security_code: String,
) -> Result<BackfillJob, StartJobError> {
    // `input` 用於 job 查詢顯示，closure 則保留原始 security_code 供實際回補使用。
    let input = security_code.clone();
    start_job(
        BACKFILL_STATE.clone(),
        "received_dividend_records",
        input,
        move || async move {
            let summary =
                dividend_record::backfill_received_dividend_records_for_stock(&security_code)
                    .await?;
            Ok(format!(
                "received dividend records backfill completed: holding_count={}, year_count={}, recalculated_count={}",
                summary.holding_count, summary.year_count, summary.recalculated_count
            ))
        },
    )
    .await
}

/// 建立歷年股利補抓背景 job。
///
/// Job 會從 Yahoo 股利頁重新抓取單一證券的歷年配息資料並 upsert 回資料庫。
pub(crate) async fn start_historical_dividends_job(
    security_code: String,
) -> Result<BackfillJob, StartJobError> {
    // `input` 用於 job 查詢顯示，closure 則保留原始 security_code 供實際回補使用。
    let input = security_code.clone();
    start_job(
        BACKFILL_STATE.clone(),
        "historical_dividends",
        input,
        move || async move {
            let upserted_count =
                dividend::backfill_historical_dividends_for_stock(&security_code).await?;
            Ok(format!(
                "historical dividends backfill completed: upserted_count={upserted_count}"
            ))
        },
    )
    .await
}

/// 建立歷史日報價回補背景 job。
///
/// Job 會對每個「代號 × 月份」呼叫 TWSE 個股月行情，寫入時只補空位
/// （`ON CONFLICT DO NOTHING`），既有資料不覆寫也不刪除，因此可安全重跑。
/// 因請求量大，改用 [`LONG_RUNNING_JOB_TIMEOUT`]。
pub(crate) async fn start_quote_history_job(
    stock_symbols: Vec<String>,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<BackfillJob, StartJobError> {
    // input 同時是查重鍵：同一組代號與區間不允許併行兩份。
    let input = format!(
        "{}|{}~{}",
        if stock_symbols.is_empty() {
            "all-etf".to_string()
        } else {
            stock_symbols.join(",")
        },
        from.format("%Y-%m"),
        to.format("%Y-%m")
    );

    start_job_with_timeout(
        BACKFILL_STATE.clone(),
        "quote_history",
        input,
        LONG_RUNNING_JOB_TIMEOUT,
        move || async move {
            // 空清單代表「全部未下市的 00 開頭 ETF／ETN」，於 job 內解析，
            // 讓新掛牌的標的自動納入。
            let symbols = if stock_symbols.is_empty() {
                quote_history::fetch_etf_symbols().await?
            } else {
                stock_symbols
            };
            let summary = quote_history::execute(&symbols, from, to).await?;
            Ok(format!(
                "quote history backfill completed: symbols={}, months_requested={}, months_with_data={}, months_failed={}, quotes_fetched={}, rows_inserted={}, rows_recalculated={}",
                symbols.len(),
                summary.months_requested,
                summary.months_with_data,
                summary.months_failed,
                summary.quotes_fetched,
                summary.rows_inserted,
                summary.rows_recalculated
            ))
        },
    )
    .await
}

/// 建立指定基準日的 CAGR 重算背景 job。
///
/// `date` 為 `None` 時採用資料庫中最新的交易日。計算只讀既有報價與股利，
/// 不呼叫外部網站；同一 `(基準日, 股票, 期間)` 為冪等的 upsert 覆蓋。
pub(crate) async fn start_cagr_job(date: Option<NaiveDate>) -> Result<BackfillJob, StartJobError> {
    let input = date.map_or_else(|| "latest".to_string(), |value| value.to_string());

    start_job(
        BACKFILL_STATE.clone(),
        "cagr",
        input,
        move || async move {
            let summary = cagr::execute(date).await?;
            Ok(format!(
                "cagr calculation completed: date={:?}, universe={}, periods_calculated={}, periods_skipped={}, rows_written={}, anomaly_symbols={}",
                summary.date,
                summary.universe,
                summary.periods_calculated,
                summary.periods_skipped,
                summary.rows_written,
                summary.anomaly_symbols
            ))
        },
    )
    .await
}

/// 建立單一統計期間的歷史回填背景 job。
///
/// 新增期間（例如 Y7）後專用：掃出「已有計算結果、但缺少該期間」的基準日
/// 逐日補算該期間。待回填的日期可能很多，改用
/// [`LONG_RUNNING_JOB_TIMEOUT`]。
pub(crate) async fn start_cagr_period_job(
    period: CagrPeriod,
) -> Result<BackfillJob, StartJobError> {
    start_job_with_timeout(
        BACKFILL_STATE.clone(),
        "cagr_period",
        period.code().to_string(),
        LONG_RUNNING_JOB_TIMEOUT,
        move || async move {
            let summary = cagr::backfill_period(period).await?;
            Ok(format!(
                "cagr period backfill completed: period={}, dates_pending={}, dates_processed={}, dates_skipped={}, rows_written={}",
                period.code(),
                summary.dates_pending,
                summary.dates_processed,
                summary.dates_skipped,
                summary.rows_written
            ))
        },
    )
    .await
}

#[cfg(test)]
mod tests;
