//! 手動回補 job 的共用流程：查重、併行閘門、登記狀態、背景執行與逾時，以及進度回報。

use super::*;

/// 單一 manual backfill job 的最長執行時間。
///
/// 批次回補（例如多檔股票逐一向 Yahoo 抓歷年股利）可能要跑數十分鐘，
/// 因此上限放寬到一小時；超過即視為異常（例如外部網站掛住、迴圈卡死），
/// 將 job 標記為 failed 並釋放併行名額，避免殭屍 job 永久佔用資源。
/// 資料寫入已採「單一 transaction 原子替換」，中斷只會 rollback，不會留下半套資料。
pub(super) const JOB_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// 歷史報價回補專用的逾時上限。
///
/// 這類 job 是「代號數 × 月份數」次外部請求，每次之間還刻意間隔 1.2 秒避免
/// 被 TWSE 限流；全部 ETF 補七年約兩萬次請求要跑數小時，套用一小時的預設
/// 逾時只會在半途被砍掉。
pub(super) const LONG_RUNNING_JOB_TIMEOUT: Duration = Duration::from_secs(12 * 60 * 60);

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
pub(super) async fn start_job<F, Fut>(
    state: BackfillWebState,
    kind: &'static str,
    input: String,
    run: F,
) -> Result<BackfillJob, StartJobError>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<String>> + Send + 'static,
{
    start_job_with_timeout(state, kind, input, JOB_TIMEOUT, run).await
}

/// 與 [`start_job`] 相同，但可指定逾時上限。
///
/// 逾時是「這個 job 合理跑多久」的宣告，不同工作差異可以很大：抓一天的收盤
/// 報價幾秒就好，補七年的個股月行情要數小時。用同一個值只能遷就最慢的那個，
/// 反而讓真正卡住的 job 拖著名額不放。
pub(super) async fn start_job_with_timeout<F, Fut>(
    state: BackfillWebState,
    kind: &'static str,
    input: String,
    timeout: Duration,
    run: F,
) -> Result<BackfillJob, StartJobError>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<String>> + Send + 'static,
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
        progress_percent: 0,
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
    let _ = state.events.send(());

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
        let result = match JOB_PROGRESS
            .scope(
                (state.clone(), job_id.clone()),
                tokio::time::timeout(timeout, run()),
            )
            .await
        {
            Ok(result) => result,
            Err(_elapsed) => Err(anyhow!("job timed out after {} seconds", timeout.as_secs())),
        };
        // 取得寫鎖後更新 job 狀態；鎖只包住狀態更新，避免長時間持鎖執行回補。
        let mut jobs = jobs.write().await;
        if let Some(job) = jobs.get_mut(&job_id) {
            job.finished_at = Some(Local::now().to_rfc3339());

            match result {
                Ok(message) => {
                    // 成功時標記 succeeded，並把回補摘要提供給 UI/API 顯示。
                    job.status = BackfillJobStatus::Succeeded;
                    job.progress_percent = 100;
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
        drop(jobs);
        let _ = state.events.send(());
    });

    Ok(job)
}

tokio::task_local! {
    static JOB_PROGRESS: (BackfillWebState, String);
}

/// 回報實際完成的階段；只在目前回補 task 的作用域內更新，不影響一般排程。
pub(super) async fn report_progress(percent: u8, message: &str) {
    let Ok((state, id)) = JOB_PROGRESS.try_with(Clone::clone) else {
        return;
    };
    if let Some(job) = state.jobs.write().await.get_mut(&id) {
        job.progress_percent = job.progress_percent.max(percent.min(99));
        job.message = message.to_string();
    }
    let _ = state.events.send(());
}
