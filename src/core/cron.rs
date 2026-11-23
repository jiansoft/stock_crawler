//! 以臺灣時間（UTC+8）解讀 cron 表達式的排程器，取代依賴 chrono 的 tokio-cron-scheduler。
//!
//! 表達式含秒欄位（`秒 分 時 日 月 星期`），日與星期同時指定時兩者都要符合，與原本
//! tokio-cron-scheduler 的設定相同。每個任務由一個背景迴圈等到下次觸發時間，觸發時另開
//! task 執行；任務執行得比間隔久時，下一輪照常開始（與原本相同）。

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use croner::Cron;
use croner::parser::{CronParser, Seconds};
use jiff::{Timestamp, ZonedDateTime};
use tokio::task::JoinHandle;

use crate::core::shutdown::BACKGROUND_OPERATIONS;
use crate::core::util::datetime::taipei;

/// 一次觸發要執行的工作。
type JobFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
/// 每次觸發時建立新 future 的工作函式。
type JobFn = dyn Fn() -> JobFuture + Send + Sync;

/// 等待下次觸發時每次最多睡這麼久，醒來重新比對牆上時間；主機校時或休眠後不會錯過太久。
const MAX_SLEEP: Duration = Duration::from_secs(60);

/// 一個排程任務：名稱、cron 表達式與每次觸發要執行的工作。
#[derive(Clone)]
pub struct CronJob {
    name: &'static str,
    expression: &'static str,
    schedule: Cron,
    task: Arc<JobFn>,
}

impl std::fmt::Debug for CronJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CronJob")
            .field("name", &self.name)
            .field("expression", &self.expression)
            .finish_non_exhaustive()
    }
}

impl CronJob {
    /// 解析 cron 表達式並建立任務。
    ///
    /// # Errors
    ///
    /// 表達式無法解析（欄位數不對、名稱拼錯、超出範圍）時回傳錯誤。
    pub fn new<F, Fut>(expression: &'static str, name: &'static str, task: F) -> Result<Self>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let schedule = CronParser::builder()
            .seconds(Seconds::Required)
            .dom_and_dow(true)
            .build()
            .parse(expression)
            .with_context(|| format!("無法解析排程 [{name}] 的 cron 表達式 '{expression}'"))?;
        Ok(Self {
            name,
            expression,
            schedule,
            task: Arc::new(move || Box::pin(task()) as JobFuture),
        })
    }

    /// 任務名稱。
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// cron 表達式。
    #[must_use]
    pub fn expression(&self) -> &'static str {
        self.expression
    }

    /// `after` 之後（不含）的下一次觸發時間，以臺灣時間表示。
    ///
    /// # Errors
    ///
    /// 之後再也不會觸發（例如指定了不存在的日期）時回傳錯誤。
    pub fn next_after(&self, after: Timestamp) -> Result<ZonedDateTime> {
        self.schedule
            .find_next_occurrence(&after.to_zoned_datetime(taipei()), false)
            .with_context(|| format!("排程 [{}] 找不到下一次觸發時間", self.name))
    }
}

/// 排程器：登記任務後 [`start`](Self::start)，關機時 [`shutdown`](Self::shutdown) 停止之後的觸發。
#[derive(Debug, Default)]
pub struct CronScheduler {
    jobs: Mutex<Vec<CronJob>>,
    loops: Mutex<Vec<JoinHandle<()>>>,
}

impl CronScheduler {
    /// 建立空的排程器。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 登記任務；[`start`](Self::start) 之後登記的任務不會執行。
    pub fn add(&self, job: CronJob) {
        lock(&self.jobs).push(job);
    }

    /// 為每個已登記的任務啟動背景迴圈。重複呼叫只會啟動新登記的任務。
    pub fn start(&self) {
        let jobs = std::mem::take(&mut *lock(&self.jobs));
        let mut loops = lock(&self.loops);
        for job in jobs {
            loops.push(tokio::spawn(drive(job)));
        }
    }

    /// 停止之後的觸發；已經開始執行的任務不受影響（由 `BACKGROUND_OPERATIONS` 等待）。
    pub fn shutdown(&self) {
        for handle in lock(&self.loops).drain(..) {
            handle.abort();
        }
    }
}

/// 取得鎖；持鎖期間只做 `Vec` 操作，不會 panic 而汙染鎖。
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .expect("排程器的鎖只在 Vec 操作期間持有，不會被 panic 汙染")
}

/// 一個任務的背景迴圈：等到觸發時間就另開 task 執行，再計算下一次。
async fn drive(job: CronJob) {
    let mut next = match job.next_after(Timestamp::now()) {
        Ok(next) => next.timestamp(),
        Err(why) => {
            tracing::error!(
                name = job.name,
                cron = job.expression,
                "排程不會再觸發: {why:#}"
            );
            return;
        }
    };
    loop {
        let now = Timestamp::now();
        if now < next {
            let wait = Duration::try_from(next.duration_since(now)).unwrap_or(MAX_SLEEP);
            tokio::time::sleep(wait.min(MAX_SLEEP)).await;
            continue;
        }
        // 先登記進行中再 spawn：關機時不會漏等已觸發但尚未開始的任務。
        let guard = BACKGROUND_OPERATIONS.begin();
        let run = (job.task)();
        tokio::spawn(async move {
            let _guard = guard;
            run.await;
        });
        // 錯過多次（例如主機休眠）只補跑一次，下一次從現在算起。
        next = match job.next_after(now) {
            Ok(next) => next.timestamp(),
            Err(why) => {
                tracing::error!(
                    name = job.name,
                    cron = job.expression,
                    "排程不會再觸發: {why:#}"
                );
                return;
            }
        };
    }
}

#[cfg(test)]
mod tests;
