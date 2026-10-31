//! 台北時區的完整排程清單；名稱、cron 與建構函式放在同一個定義，避免註冊與測試脫節。
//!
//! 依時段分成子模組，每個子模組內依執行時間排序。
use anyhow::Result;
use tokio_cron_scheduler::Job;

/// 一個排程的固定中繼資料與不執行任務的建構函式。
pub(super) struct JobDefinition {
    pub(super) cron: &'static str,
    pub(super) name: &'static str,
    pub(super) create: fn() -> Result<Job>,
}

/// 將任務函式與固定中繼資料綁在一起，保留一般與靜默兩種記錄模式。
macro_rules! job {
    (normal, $cron:literal, $name:literal, $task:path $(,)?) => {
        JobDefinition {
            cron: $cron,
            name: $name,
            create: || $crate::app::scheduler::create_job($cron, $name, $task),
        }
    };
    (quiet, $cron:literal, $name:literal, $task:path $(,)?) => {
        JobDefinition {
            cron: $cron,
            name: $name,
            create: || $crate::app::scheduler::create_quiet_job($cron, $name, $task),
        }
    };
}

mod evening;
mod monitoring;
mod overnight;
mod trading;
mod weekly;

/// 全部排程：盤前批次、開盤前後、盤後、週六、固定間隔監控；建構不觸發任何採集或通知。
pub(super) fn definitions() -> Vec<JobDefinition> {
    let groups = [
        overnight::definitions(),
        trading::definitions(),
        evening::definitions(),
        weekly::definitions(),
        monitoring::definitions(),
    ];
    groups.into_iter().flatten().collect()
}
