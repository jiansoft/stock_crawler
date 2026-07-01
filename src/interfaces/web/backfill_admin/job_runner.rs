use std::sync::Arc;

use anyhow::{Result, anyhow};
use axum::Json;
use axum::response::IntoResponse;
use chrono::{Local, NaiveDate};

use crate::{
    app::backfill::{dividend, quote, taiwan_stock_index},
    app::calculation::dividend_record,
    app::event::taiwan_stock::closing,
};

use super::dto::ErrorResponse;
use super::state::{BACKFILL_STATE, BackfillJob, BackfillJobStatus, BackfillWebState};

/// 建立各股每日收盤報價背景 job。
///
/// Job 會先刪除指定交易日既有的 `DailyQuotes`，再呼叫 TWSE 與 TPEx 來源重抓
/// 上市櫃各股每日開高低收、成交量與本益比等資料。
pub(crate) async fn start_daily_quotes_job(date: NaiveDate) -> BackfillJob {
    start_job(
        BACKFILL_STATE.clone(),
        "daily_quotes",
        date.to_string(),
        move || async move {
            // quote::execute 使用 COPY 寫入 DailyQuotes；先清掉同日資料可避免唯一索引衝突，
            // 也讓手動回補確實以外部來源的最新內容重建當日各股收盤報價。
            use crate::domain::quote::repository::QuoteRepository;
            let quote_repo = crate::infra::database::repository::quote::PgQuoteRepository::new();
            quote_repo.delete_quotes_by_date(date).await?;

            let quote_count = quote::execute(date).await?;
            Ok(format!(
                "daily quotes backfill completed: quote_count={quote_count}"
            ))
        },
    )
    .await
}

/// 建立多次配息股票歷年股利批次回補背景 job。
///
/// Job 會找出指定年度已有季配/半年配資料的股票，逐檔重新抓取 Yahoo 歷年股利，
/// 並把成功處理的股票數與明細 upsert 筆數寫入 job message。
pub(crate) async fn start_multiple_dividend_historical_dividends_job(year: i32) -> BackfillJob {
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

/// 建立收盤彙總背景 job。
///
/// Job 會呼叫 `closing::aggregate` 重新彙總指定交易日的收盤資料。
pub(crate) async fn start_closing_aggregate_job(date: NaiveDate) -> BackfillJob {
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
pub(crate) async fn start_taiwan_stock_index_job(date: NaiveDate) -> BackfillJob {
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
pub(crate) async fn start_received_dividend_records_job(security_code: String) -> BackfillJob {
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
pub(crate) async fn start_historical_dividends_job(security_code: String) -> BackfillJob {
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

/// 建立並啟動一個 manual backfill 背景 job。
///
/// 此 helper 封裝共用流程：建立初始 job、寫入記憶體狀態、spawn 背景 task、
/// 依執行結果更新 job 狀態與 log。
async fn start_job<F, Fut>(
    state: BackfillWebState,
    kind: &'static str,
    input: String,
    run: F,
) -> BackfillJob
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<String>> + Send + 'static,
{
    // 先建立 running 狀態的 job，讓呼叫端能立即取得 id。
    let job = BackfillJob {
        id: state.next_job_id(),
        kind: kind.to_string(),
        input,
        status: BackfillJobStatus::Running,
        message: "queued".to_string(),
        started_at: Local::now().to_rfc3339(),
        finished_at: None,
    };
    // job_id 用於背景 task 完成時回頭更新同一筆 job。
    let job_id = job.id.clone();
    // task_job 保留啟動 log 需要的資訊，避免 move 掉回傳用的 job。
    let task_job = job.clone();

    // 寫入全域 job 表後再 spawn，確保呼叫端一拿到 id 就查得到。
    state.jobs.write().await.insert(job_id.clone(), job.clone());

    // 背景執行實際回補工作，HTTP/gRPC 呼叫端只負責後續輪詢。
    let jobs = Arc::clone(&state.jobs);
    tokio::spawn(async move {
        tracing::info!(
            "manual backfill job started: id={}, kind={}, input={}",
            task_job.id,
            task_job.kind,
            task_job.input
        );

        // 執行呼叫端提供的回補流程，成功時回傳完成訊息，失敗時保留錯誤鏈。
        let result = run().await;
        // 取得寫鎖後更新 job 狀態；鎖只包住狀態更新，避免長時間持鎖執行回補。
        let mut jobs = jobs.write().await;
        if let Some(job) = jobs.get_mut(&job_id) {
            job.finished_at = Some(Local::now().to_rfc3339());

            match result {
                Ok(message) => {
                    // 成功時標記 succeeded，並把回補摘要提供給 UI/API 顯示。
                    job.status = BackfillJobStatus::Succeeded;
                    job.message = message;
                    tracing::info!(
                        "manual backfill job succeeded: id={}, kind={}, input={}, message={}",
                        job.id,
                        job.kind,
                        job.input,
                        job.message
                    );
                }
                Err(why) => {
                    // 失敗時標記 failed，message 使用 anyhow 的完整錯誤鏈。
                    job.status = BackfillJobStatus::Failed;
                    job.message = format!("{why:#}");
                    tracing::error!(
                        "manual backfill job failed: id={}, kind={}, input={}, error={:#}",
                        job.id,
                        job.kind,
                        job.input,
                        why
                    );
                }
            }
        }
    });

    job
}

/// 解析 HTTP request 的日期欄位，格式錯誤時回傳一致的 400 response。
#[allow(clippy::result_large_err)]
pub(super) fn parse_request_date(date: &str) -> Result<NaiveDate, axum::response::Response> {
    NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d").map_err(|why| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("date must use YYYY-MM-DD: {why}"),
            }),
        )
            .into_response()
    })
}

/// 解析 HTTP request 的證券代號欄位，格式錯誤時回傳一致的 400 response。
#[allow(clippy::result_large_err)]
pub(super) fn parse_request_security_code(
    security_code: String,
) -> Result<String, axum::response::Response> {
    normalize_security_code(security_code).map_err(|why| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: why.to_string(),
            }),
        )
            .into_response()
    })
}

/// 正規化並驗證證券代號。
///
/// 此函式會 trim 頭尾空白，拒絕空字串與非 ASCII 英數字元，成功時回傳清理後的代號。
pub(crate) fn normalize_security_code(security_code: String) -> Result<String> {
    // 移除表單或 API 呼叫常見的頭尾空白。
    let security_code = security_code.trim();
    // 空代號無法執行任何回補流程，直接回報輸入錯誤。
    if security_code.is_empty() {
        return Err(anyhow!("security_code is required"));
    }
    // 僅允許英數代號，避免把符號或空白帶入 crawler/database 查詢。
    if !security_code.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Err(anyhow!(
            "security_code may only contain ASCII letters or numbers"
        ));
    }
    // 回傳擁有權字串，讓後續 async job 可以安全 move 進背景 task。
    Ok(security_code.to_string())
}
