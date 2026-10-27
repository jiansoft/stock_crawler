//! 排程啟動、逾時控制與執行機制；任務清單見 [`jobs`]。

mod jobs;

use std::{
    env,
    future::Future,
    time::{Duration, Instant},
};

use anyhow::{Context, Error, Result};
use chrono::FixedOffset;
use tokio_cron_scheduler::{Job, JobScheduler};

use crate::{
    app::event,
    // 通知一律走 core::alert 抽象介面（port）：app 層不 import
    // interfaces::bot（傳輸層細節），實際的 Telegram adapter 由 main 啟動時註冊。
    // 這樣 use case 可以在單元測試中注入假的 sink 驗證，也能日後替換通知管道。
    core::{alert, declare, util::text},
};

/// 啟動排程
pub async fn start(sched: &JobScheduler) -> Result<()> {
    let timer = Instant::now();
    tracing::info!("scheduler start begin: run_cron");
    let run_cron_timer = Instant::now();
    run_cron(sched).await.context("Failed to run cron jobs")?;
    tracing::info!(
        "scheduler start done: run_cron elapsed={:?}",
        run_cron_timer.elapsed()
    );

    //若在開盤埘間重啟服務定時任務會無法觸發，所以在啟動時要先執行股價追踪的任務，執行完後再設定一次定時任務
    if declare::StockExchange::TWSE.is_open() {
        tracing::info!("scheduler start begin: opening trace::stock_price");
        let opening_trace_timer = Instant::now();
        if let Err(why) = event::trace::stock_price::execute().await {
            let err_msg = format!("{:?}", why);
            tracing::error!("{}", &err_msg);
            alert::send_alert("開盤股價追蹤初始化失敗", &err_msg).await;
        }
        tracing::info!(
            "scheduler start done: opening trace::stock_price elapsed={:?}",
            opening_trace_timer.elapsed()
        );
    }

    let msg = format!(
        "StockCrawler 已啟動\r\nRust OS/Arch: {}/{}\r\n",
        // OS/Arch 是動態內容，照規則先做 MarkdownV2 跳脫再組進訊息。
        text::escape_markdown_v2(env::consts::OS),
        text::escape_markdown_v2(env::consts::ARCH)
    );

    tracing::info!("scheduler start begin: telegram notify");
    let telegram_timer = Instant::now();
    alert::send_message(&msg).await;
    tracing::info!(
        "scheduler start done: telegram notify elapsed={:?}",
        telegram_timer.elapsed()
    );
    tracing::info!("scheduler start done: total elapsed={:?}", timer.elapsed());

    Ok(())
}

/// 註冊所有 cron 任務並啟動排程器。
async fn run_cron(sched: &JobScheduler) -> Result<()> {
    let timer = Instant::now();
    //let sched = JobScheduler::new().await?;
    //                 sec  min   hour   day of month   month   day of week   year
    //let expression = "0   30   9,12,15     1,15       May-Aug  Mon,Wed,Fri  2018/2";
    // 台北時間（UTC+8）：create_job 內以 Job::new_async_tz 搭配 FixedOffset(+8)
    // 解讀 cron 表達式，因此以下註解標示的時間皆為台北時間。

    let mut job_count = 0usize;
    for definition in jobs::definitions() {
        // 沿用拆檔前 Result::into_iter().flatten() 的行為：無效定義略過，其餘照常註冊。
        let Ok(job) = (definition.create)() else {
            tracing::debug!(
                name = definition.name,
                cron = definition.cron,
                "略過無效排程定義"
            );
            continue;
        };
        sched
            .add(job)
            .await
            .context("Failed to add job to scheduler")?;
        job_count += 1;
    }

    sched.start().await.context("Failed to start scheduler")?;
    tracing::info!(
        "scheduler run_cron done: jobs={}, elapsed={:?}",
        job_count,
        timer.elapsed()
    );

    Ok(())
}

/// 排程輔助介面。
pub trait Scheduler {
    /// 判斷目前時間是否為週末。
    fn is_weekend(&self) -> bool;
}

/// 將非同步工作包裝成 `tokio_cron_scheduler::Job`，並自動記錄執行時間。
///
/// 每次任務註冊與觸發時會：
/// 1. 於啟動註冊階段記錄排程任務名稱與運作週期。
/// 2. 觸發時記錄 `task.begin` 事件（含 cron 表達式與任務名稱作為結構化欄位）。
/// 3. 執行任務並計時。
/// 4. 記錄 `task.done` 事件（含 `elapsed_ms`）或 `task.failed` 事件（含錯誤訊息）。
/// 5. 超過 [`JOB_TIMEOUT`] 仍未結束時中止該輪，記錄 `task.timeout` 並發送告警。
///
/// 結構化欄位（`task`、`name`、`elapsed_ms`）透過 F1 `FieldCollector` 同步送到 Seq，
/// 讓 ops 可直接在 Seq 查詢特定任務的執行時間趨勢。
fn create_job<F, Fut>(cron_expr: &'static str, name: &'static str, task: F) -> Result<Job>
where
    F: Fn() -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<(), Error>> + Send + 'static,
{
    register_job(cron_expr, name, false, task)
}

/// 同 [`create_job`]，但 `task.begin`／`task.done` 記為 DEBUG。
///
/// 給每幾分鐘就跑一次的任務：一天上百輪的開始／結束紀錄會淹沒 info 日誌。失敗與逾時照常記 ERROR 並告警。
fn create_quiet_job<F, Fut>(cron_expr: &'static str, name: &'static str, task: F) -> Result<Job>
where
    F: Fn() -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<(), Error>> + Send + 'static,
{
    register_job(cron_expr, name, true, task)
}

/// [`create_job`]／[`create_quiet_job`] 的共同實作。
fn register_job<F, Fut>(
    cron_expr: &'static str,
    name: &'static str,
    quiet: bool,
    task: F,
) -> Result<Job>
where
    F: Fn() -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<(), Error>> + Send + 'static,
{
    tracing::info!("註冊排程任務: [{}] 運作週期: '{}'", name, cron_expr);
    let tz = FixedOffset::east_opt(8 * 3600).unwrap();
    Ok(Job::new_async_tz(cron_expr, tz, move |_uuid, _l| {
        let task = task.clone();
        Box::pin(run_job(cron_expr, name, quiet, JOB_TIMEOUT, task))
    })?)
}

/// 執行一輪排程任務：登記為進行中、計時、記錄結果，失敗或逾時時告警。
async fn run_job<F, Fut>(
    cron_expr: &'static str,
    name: &'static str,
    quiet: bool,
    limit: Duration,
    task: F,
) where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<(), Error>>,
{
    // 排程一旦「真正開始執行」才登記為 active，主程式關機時會等待此 use case 離開。
    // guard 必須在這裡取得，不能放在 create_job 階段；後者只是在啟動時
    // 註冊排程，若那時就 +1，counter 會在整個服務生命週期永遠無法歸零。
    let _operation_guard = crate::core::shutdown::BACKGROUND_OPERATIONS.begin();
    if quiet {
        tracing::debug!(task = cron_expr, name = name, "task.begin");
    } else {
        tracing::info!(task = cron_expr, name = name, "task.begin");
    }
    let t = Instant::now();
    match run_with_timeout(task(), limit).await {
        TaskOutcome::Done => {
            let elapsed_ms = t.elapsed().as_millis() as u64;
            if quiet {
                tracing::debug!(task = cron_expr, name = name, elapsed_ms, "task.done");
            } else {
                tracing::info!(task = cron_expr, name = name, elapsed_ms, "task.done");
            }
        }
        TaskOutcome::Failed(why) => {
            let err_msg = format!("{:?}", why);
            tracing::error!(
                task = cron_expr,
                name = name,
                elapsed_ms = t.elapsed().as_millis() as u64,
                error = %err_msg,
                "task.failed"
            );
            alert::send_alert(&format!("排程任務 [{}] 執行失敗", name), &err_msg).await;
        }
        TaskOutcome::TimedOut => {
            let err_msg = format!(
                "執行超過 {} 分鐘仍未結束，已中止本輪；請查看日誌中該任務最後的紀錄",
                limit.as_secs() / 60
            );
            tracing::error!(
                task = cron_expr,
                name = name,
                elapsed_ms = t.elapsed().as_millis() as u64,
                "task.timeout"
            );
            alert::send_alert(&format!("排程任務 [{}] 逾時", name), &err_msg).await;
        }
    }
}

/// 單輪排程任務的執行時間上限。
///
/// 沒有上限時，任務只要卡住就永遠不會留下 `task.done`／`task.failed`，也不會告警：
/// 2026-09-21 21:00 的股利排程因重試延遲失控（第四次重試要等 24.75 小時）卡了兩天，
/// 直到重新部署才被發現，期間隔天同時段又啟動了新的一輪。
///
/// 3 小時的依據：正式站 2026-09-17～24 各排程最長 63.9 分鐘（21:00 股利採集，
/// 且為修正重試前的數字），Yahoo 財報採集預估約 45 分鐘。上限小於 24 小時，
/// 也保證每日排程不會兩輪重疊。
const JOB_TIMEOUT: Duration = Duration::from_secs(3 * 60 * 60);

/// 單輪排程任務的執行結果。
#[derive(Debug)]
enum TaskOutcome {
    /// 正常結束。
    Done,
    /// 任務回傳錯誤。
    Failed(Error),
    /// 超過時間上限，已中止。
    TimedOut,
}

/// 在 `limit` 內執行任務；逾時即 drop 任務 future 將其中止。
///
/// 中止發生在任務的下一個 `.await` 點：進行中的資料庫交易隨 drop 回滾，
/// 已提交的寫入保留。採集類任務皆為 upsert／逐檔處理，下一輪可以接續。
async fn run_with_timeout<Fut>(task: Fut, limit: Duration) -> TaskOutcome
where
    Fut: Future<Output = Result<(), Error>>,
{
    match tokio::time::timeout(limit, task).await {
        Ok(Ok(())) => TaskOutcome::Done,
        Ok(Err(why)) => TaskOutcome::Failed(why),
        Err(_) => TaskOutcome::TimedOut,
    }
}

#[cfg(test)]
mod tests;
