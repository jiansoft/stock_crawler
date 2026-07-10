use std::sync::Arc;
use std::time::Duration;

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
use super::state::{
    BACKFILL_STATE, BackfillJob, BackfillJobStatus, BackfillWebState, MAX_CONCURRENT_JOBS,
};

/// 單一 manual backfill job 的最長執行時間。
///
/// 批次回補（例如多檔股票逐一向 Yahoo 抓歷年股利）可能要跑數十分鐘，
/// 因此上限放寬到一小時；超過即視為異常（例如外部網站掛住、迴圈卡死），
/// 將 job 標記為 failed 並釋放併行名額，避免殭屍 job 永久佔用資源。
/// 資料寫入已採「單一 transaction 原子替換」，中斷只會 rollback，不會留下半套資料。
const JOB_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// 建立 manual backfill job 失敗的原因。
///
/// 這是「拒絕建立」的錯誤，不是 job 執行失敗——job 執行失敗會反映在
/// job 本身的 `failed` 狀態上。HTTP 與 gRPC 各自把這個 enum 轉成
/// 對應的錯誤碼（409/429、ALREADY_EXISTS/RESOURCE_EXHAUSTED）。
#[derive(Debug)]
pub(crate) enum StartJobError {
    /// 相同 `(kind, input)` 已有一個執行中的 job。
    ///
    /// 兩份相同的回補同時跑會互相刪除/覆寫資料，因此直接拒絕，
    /// 並附上既有 job 的 id 讓呼叫端可以改為輪詢它。
    DuplicateActiveJob {
        /// 既有執行中 job 的 id。
        existing_id: String,
    },
    /// 執行中的 job 總數已達全域上限。
    TooManyActiveJobs {
        /// 目前設定的併行上限，用於錯誤訊息提示。
        max: usize,
    },
}

impl std::fmt::Display for StartJobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartJobError::DuplicateActiveJob { existing_id } => write!(
                f,
                "the same backfill job is already running: id={existing_id}"
            ),
            StartJobError::TooManyActiveJobs { max } => write!(
                f,
                "too many backfill jobs are running (limit: {max}), try again later"
            ),
        }
    }
}

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

/// 建立並啟動一個 manual backfill 背景 job。
///
/// 此 helper 封裝共用流程，依序做四件事：
///
/// 1. **查重**：相同 `(kind, input)` 已有執行中的 job 時拒絕建立，
///    避免兩份相同回補互相刪除/覆寫資料。
/// 2. **併行閘門**：以 semaphore 限制同時執行的 job 總數
///    （[`MAX_CONCURRENT_JOBS`]），滿了直接拒絕而不是排隊。
/// 3. **登記與修剪**：建立 running 狀態的 job 寫入記憶體狀態表，
///    並順手修剪過舊的已完成 job，讓狀態表大小有上限。
/// 4. **背景執行**：spawn task 執行回補（帶 [`JOB_TIMEOUT`] 逾時保護），
///    完成後回頭更新 job 狀態與 log。
///
/// 查重、領名額、登記三步都發生在同一次 `jobs` 寫鎖期間——
/// 這保證兩個並發請求不可能同時通過查重（check-then-act 競態）。
async fn start_job<F, Fut>(
    state: BackfillWebState,
    kind: &'static str,
    input: String,
    run: F,
) -> Result<BackfillJob, StartJobError>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<String>> + Send + 'static,
{
    // 取得寫鎖。從這裡到 drop(jobs_guard) 之間的檢查與登記是原子的：
    // 其他請求必須等鎖釋放才能做自己的查重，因此不會有兩個相同 job 同時通過。
    let mut jobs_guard = state.jobs.write().await;

    // 防護 1：相同 (kind, input) 只允許一個執行中的 job。
    if let Some(existing_id) = BackfillWebState::find_active_job(&jobs_guard, kind, &input) {
        return Err(StartJobError::DuplicateActiveJob { existing_id });
    }

    // 防護 2：全域併行上限。try_acquire 立即返回，不會讓 HTTP request 掛著等。
    // permit 是 RAII 憑證，稍後 move 進背景 task；task 結束（成功、失敗、逾時、
    // panic）時 permit 被 drop，名額自動歸還。
    let permit = state
        .try_acquire_job_slot()
        .ok_or(StartJobError::TooManyActiveJobs {
            max: MAX_CONCURRENT_JOBS,
        })?;

    // 通過兩道防護後才建立 running 狀態的 job，讓呼叫端能立即取得 id。
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
    jobs_guard.insert(job_id.clone(), job.clone());
    // 防護 3：趁著已持有寫鎖，修剪超過保留上限的已完成 job，
    // 讓長時間運行的程序記憶體不會被歷史 job 無限撐大。
    BackfillWebState::prune_finished_jobs(&mut jobs_guard);
    // 登記完成，儘早釋放寫鎖；背景 task 的實際回補不應在持鎖狀態下執行。
    drop(jobs_guard);

    // 背景執行實際回補工作，HTTP/gRPC 呼叫端只負責後續輪詢。
    let jobs = Arc::clone(&state.jobs);
    // 在 spawn 前就登記，而不是進入 async block 後才登記。
    // 原因是 tokio::spawn 只把 future 放進排程佇列，不保證立刻 poll；若 OS 訊號恰好在
    // 兩者之間抵達，main 可能看到 active=0 而提早退出。先取得 guard 可關閉這個競態窗口。
    let operation_guard = crate::core::shutdown::BACKGROUND_OPERATIONS.begin();
    tokio::spawn(async move {
        // 把 guard 與 permit move 進 task 並綁在整個 scope。以下 run().await 不論成功、
        // 錯誤、逾時或 panic unwind，Rust 都會 drop 這兩個 RAII 物件：
        // 關機 counter 不會永久卡住，併行名額也一定會歸還。
        let _operation_guard = operation_guard;
        let _job_slot_permit = permit;
        tracing::info!(
            "manual backfill job started: id={}, kind={}, input={}",
            task_job.id,
            task_job.kind,
            task_job.input
        );

        // 防護 4：逾時保護。tokio::time::timeout 在時限內回傳 Ok(內層結果)；
        // 超時則直接 drop 掉還在執行的 future（正在進行的資料庫 transaction
        // 會因連線歸還而自動 rollback，不會留下半套資料）並回傳 Err。
        let result = match tokio::time::timeout(JOB_TIMEOUT, run()).await {
            Ok(result) => result,
            Err(_elapsed) => Err(anyhow!(
                "job timed out after {} seconds",
                JOB_TIMEOUT.as_secs()
            )),
        };
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

    Ok(job)
}

/// 將 [`StartJobError`] 轉成一致的 HTTP 錯誤回應。
///
/// - 重複 job → `409 Conflict`：同一份工作已在執行，訊息附上既有 job id。
/// - 併行已滿 → `429 Too Many Requests`：請呼叫端稍後重試。
pub(super) fn start_job_error_response(err: StartJobError) -> axum::response::Response {
    let status = match err {
        StartJobError::DuplicateActiveJob { .. } => axum::http::StatusCode::CONFLICT,
        StartJobError::TooManyActiveJobs { .. } => axum::http::StatusCode::TOO_MANY_REQUESTS,
    };
    (
        status,
        Json(ErrorResponse {
            error: err.to_string(),
        }),
    )
        .into_response()
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::Semaphore;

    /// 輪詢等待指定 job 離開 running 狀態（最多約 5 秒）。
    async fn wait_until_finished(state: &BackfillWebState, id: &str) {
        for _ in 0..500 {
            let finished = state
                .jobs
                .read()
                .await
                .get(id)
                .map(|job| job.status_label() != "running")
                .unwrap_or(false);
            if finished {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("job {id} did not finish in time");
    }

    /// 驗證相同 (kind, input) 的 job 執行期間會被拒絕，結束後可再啟動。
    #[tokio::test]
    async fn duplicate_active_job_is_rejected() {
        let state = BackfillWebState::new();
        // gate 讓第一個 job 停在執行中，直到測試主動放行；
        // 用 Semaphore(0) 而不是 Notify，是因為 permit 可以累積，
        // 不會因 job 尚未開始等待就遺失喚醒訊號。
        let gate = Arc::new(Semaphore::new(0));

        let g = Arc::clone(&gate);
        let first = start_job(
            state.clone(),
            "test_dup",
            "2026-01-01".to_string(),
            move || {
                async move {
                    // forget()：取得 permit 後不歸還，確保一張 permit 只放行一個 job。
                    g.acquire().await.expect("gate should stay open").forget();
                    Ok("done".to_string())
                }
            },
        )
        .await
        .expect("first job should start");

        // 相同 kind + input 且第一個 job 仍在執行 → 應被拒絕並回報既有 job id。
        let err = start_job(
            state.clone(),
            "test_dup",
            "2026-01-01".to_string(),
            || async { Ok(String::new()) },
        )
        .await
        .expect_err("duplicate job should be rejected");
        match err {
            StartJobError::DuplicateActiveJob { existing_id } => {
                assert_eq!(existing_id, first.id);
            }
            other => panic!("expected DuplicateActiveJob, got {other:?}"),
        }

        // 不同 input 不算重複，可以並行。
        let second = start_job(
            state.clone(),
            "test_dup",
            "2026-01-02".to_string(),
            || async { Ok(String::new()) },
        )
        .await
        .expect("different input should start");

        // 放行第一個 job，等兩個都結束。
        gate.add_permits(1);
        wait_until_finished(&state, &first.id).await;
        wait_until_finished(&state, &second.id).await;

        // 第一個 job 已結束（不再是 active），相同 input 應可重新啟動。
        let again = start_job(
            state.clone(),
            "test_dup",
            "2026-01-01".to_string(),
            || async { Ok(String::new()) },
        )
        .await
        .expect("same input should start after previous job finished");
        wait_until_finished(&state, &again.id).await;
    }

    /// 驗證全域併行上限：滿載時拒絕新 job，名額釋放後恢復受理。
    #[tokio::test]
    async fn too_many_active_jobs_is_rejected() {
        let state = BackfillWebState::new();
        let gate = Arc::new(Semaphore::new(0));

        // 佔滿所有併行名額；每個 job 都停在 gate 上等待放行。
        let mut ids = Vec::new();
        for i in 0..MAX_CONCURRENT_JOBS {
            let g = Arc::clone(&gate);
            let job = start_job(
                state.clone(),
                "test_limit",
                format!("input-{i}"),
                move || async move {
                    g.acquire().await.expect("gate should stay open").forget();
                    Ok("done".to_string())
                },
            )
            .await
            .expect("job within limit should start");
            ids.push(job.id);
        }

        // 第 MAX_CONCURRENT_JOBS + 1 個 job → 名額已滿，應被拒絕。
        let err = start_job(
            state.clone(),
            "test_limit",
            "input-overflow".to_string(),
            || async { Ok(String::new()) },
        )
        .await
        .expect_err("job beyond limit should be rejected");
        assert!(matches!(err, StartJobError::TooManyActiveJobs { .. }));

        // 一次放行所有 job，等全部結束（permit 會被 RAII 歸還）。
        gate.add_permits(MAX_CONCURRENT_JOBS);
        for id in &ids {
            wait_until_finished(&state, id).await;
        }

        // 名額釋放後應可再受理新 job。
        let job = start_job(
            state.clone(),
            "test_limit",
            "input-after".to_string(),
            || async { Ok(String::new()) },
        )
        .await
        .expect("job should start after slots are released");
        wait_until_finished(&state, &job.id).await;
    }

    /// 驗證逾時保護：超過 JOB_TIMEOUT 的 job 會被標記為 failed。
    ///
    /// `start_paused = true` 讓 Tokio 的虛擬時鐘在所有 task 閒置時自動快轉，
    /// 因此測試不需要真的等一小時。
    #[tokio::test(start_paused = true)]
    async fn job_times_out_and_is_marked_failed() {
        let state = BackfillWebState::new();

        // 這個 job 永遠不會自己結束（pending future），只能靠逾時機制收掉。
        let job = start_job(
            state.clone(),
            "test_timeout",
            "input".to_string(),
            || async {
                std::future::pending::<()>().await;
                Ok(String::new())
            },
        )
        .await
        .expect("job should start");

        // 直接把虛擬時鐘推過逾時點，再輪詢等待背景 task 把狀態寫回。
        tokio::time::sleep(JOB_TIMEOUT + Duration::from_secs(1)).await;
        wait_until_finished(&state, &job.id).await;

        let finished = state
            .jobs
            .read()
            .await
            .get(&job.id)
            .cloned()
            .expect("job should remain queryable");
        assert_eq!(finished.status_label(), "failed");
        assert!(
            finished.message.contains("timed out"),
            "message should mention timeout: {}",
            finished.message
        );
    }
}
