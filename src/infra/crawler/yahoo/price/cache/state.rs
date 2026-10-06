//! Yahoo 類股快取背景任務的生命週期旗標與最近一輪診斷數據。

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};

use once_cell::sync::Lazy;
use tokio::task::JoinHandle;

use crate::core::util::diagnostics::{ProcessMemoryStats, TaskRuntimeStatus};

/// 控制 Yahoo 類股快取背景任務生命週期的全域旗標。
pub(super) static IS_CACHING: Lazy<AtomicBool> = Lazy::new(|| AtomicBool::new(false));
/// 目前存活中的 Yahoo 類股背景 task 數量。
pub(super) static ACTIVE_TASKS: Lazy<AtomicUsize> = Lazy::new(|| AtomicUsize::new(0));
/// Yahoo 類股背景 task 的世代編號。
pub(super) static LAST_TASK_GENERATION: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(0));
/// 目前執行中的 Yahoo 類股背景 task handle。
pub(super) static CACHING_TASK: Lazy<std::sync::Mutex<Option<JoinHandle<()>>>> =
    Lazy::new(|| std::sync::Mutex::new(None));
/// Yahoo 最近一輪成功輪詢的類股數。
pub(super) static LAST_SUCCESS_COUNT: Lazy<AtomicUsize> = Lazy::new(|| AtomicUsize::new(0));
/// Yahoo 最近一輪失敗的類股數。
pub(super) static LAST_FAILURE_COUNT: Lazy<AtomicUsize> = Lazy::new(|| AtomicUsize::new(0));
/// Yahoo 最近一輪抓取的總頁數。
pub(super) static LAST_PAGE_COUNT: Lazy<AtomicUsize> = Lazy::new(|| AtomicUsize::new(0));
/// Yahoo 最近一輪讀到的原始 item 總數。
pub(super) static LAST_RAW_ITEM_COUNT: Lazy<AtomicUsize> = Lazy::new(|| AtomicUsize::new(0));
/// Yahoo 最近一輪落地後的總快取筆數。
pub(super) static LAST_SNAPSHOT_COUNT: Lazy<AtomicUsize> = Lazy::new(|| AtomicUsize::new(0));
/// Yahoo 最近一輪候選價格事件數。
pub(super) static LAST_CANDIDATE_EVENT_COUNT: Lazy<AtomicUsize> = Lazy::new(|| AtomicUsize::new(0));
/// Yahoo 最近一輪整體耗時毫秒數。
pub(super) static LAST_ELAPSED_MS: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(0));
/// Yahoo 最近一輪 RSS 差值（KiB）。
pub(super) static LAST_RSS_DELTA_KIB: Lazy<AtomicI64> = Lazy::new(|| AtomicI64::new(0));
/// Yahoo 完成輪詢的累積輪數。
pub(super) static COMPLETED_CYCLES: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(0));

/// Yahoo 類股來源的最近一輪執行摘要。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct YahooRuntimeDiagnostics {
    pub status: TaskRuntimeStatus,
    pub last_success_count: usize,
    pub last_failure_count: usize,
    pub last_page_count: usize,
    pub last_raw_item_count: usize,
    pub last_snapshot_count: usize,
    pub last_candidate_event_count: usize,
    pub last_elapsed_ms: u64,
    pub last_rss_delta_kib: i64,
    pub completed_cycles: u64,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn store_runtime_progress(
    success_count: usize,
    failure_count: usize,
    page_count: usize,
    raw_item_count: usize,
    total_snapshot_count: usize,
    candidate_event_count: usize,
    elapsed_ms: u64,
    rss_delta_kib: i64,
) {
    LAST_SUCCESS_COUNT.store(success_count, Ordering::SeqCst);
    LAST_FAILURE_COUNT.store(failure_count, Ordering::SeqCst);
    LAST_PAGE_COUNT.store(page_count, Ordering::SeqCst);
    LAST_RAW_ITEM_COUNT.store(raw_item_count, Ordering::SeqCst);
    LAST_SNAPSHOT_COUNT.store(total_snapshot_count, Ordering::SeqCst);
    LAST_CANDIDATE_EVENT_COUNT.store(candidate_event_count, Ordering::SeqCst);
    LAST_ELAPSED_MS.store(elapsed_ms, Ordering::SeqCst);
    LAST_RSS_DELTA_KIB.store(rss_delta_kib, Ordering::SeqCst);
}

/// 取得 Yahoo 類股背景任務目前的執行狀態。
pub(crate) fn diagnostics_snapshot() -> TaskRuntimeStatus {
    TaskRuntimeStatus::new(
        IS_CACHING.load(Ordering::SeqCst),
        ACTIVE_TASKS.load(Ordering::SeqCst),
        LAST_TASK_GENERATION.load(Ordering::SeqCst),
    )
}

/// 取得 Yahoo 最近一輪輪詢的執行摘要。
pub(crate) fn runtime_diagnostics_snapshot() -> YahooRuntimeDiagnostics {
    YahooRuntimeDiagnostics {
        status: diagnostics_snapshot(),
        last_success_count: LAST_SUCCESS_COUNT.load(Ordering::SeqCst),
        last_failure_count: LAST_FAILURE_COUNT.load(Ordering::SeqCst),
        last_page_count: LAST_PAGE_COUNT.load(Ordering::SeqCst),
        last_raw_item_count: LAST_RAW_ITEM_COUNT.load(Ordering::SeqCst),
        last_snapshot_count: LAST_SNAPSHOT_COUNT.load(Ordering::SeqCst),
        last_candidate_event_count: LAST_CANDIDATE_EVENT_COUNT.load(Ordering::SeqCst),
        last_elapsed_ms: LAST_ELAPSED_MS.load(Ordering::SeqCst),
        last_rss_delta_kib: LAST_RSS_DELTA_KIB.load(Ordering::SeqCst),
        completed_cycles: COMPLETED_CYCLES.load(Ordering::SeqCst),
    }
}

pub(super) fn rss_delta_kib(
    before: Option<ProcessMemoryStats>,
    after: Option<ProcessMemoryStats>,
) -> i64 {
    match (before, after) {
        (Some(before), Some(after)) => after.vm_rss_kib as i64 - before.vm_rss_kib as i64,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 最近一輪的診斷數據寫入後讀得回來，狀態欄位與 [`diagnostics_snapshot`] 一致。
    #[test]
    fn runtime_diagnostics_snapshot_reads_the_last_progress() {
        let before = COMPLETED_CYCLES.load(Ordering::SeqCst);
        store_runtime_progress(3, 1, 7, 120, 900, 5, 1_234, -16);

        let snapshot = runtime_diagnostics_snapshot();
        assert_eq!(snapshot.status, diagnostics_snapshot());
        assert_eq!(snapshot.last_success_count, 3);
        assert_eq!(snapshot.last_failure_count, 1);
        assert_eq!(snapshot.last_page_count, 7);
        assert_eq!(snapshot.last_raw_item_count, 120);
        assert_eq!(snapshot.last_snapshot_count, 900);
        assert_eq!(snapshot.last_candidate_event_count, 5);
        assert_eq!(snapshot.last_elapsed_ms, 1_234);
        assert_eq!(snapshot.last_rss_delta_kib, -16);
        assert_eq!(snapshot.completed_cycles, before);
    }

    /// RSS 差值需要前後兩次取樣；任一次取不到就回 0。
    #[test]
    fn rss_delta_kib_needs_both_samples() {
        let sample = |vm_rss_kib| ProcessMemoryStats {
            vm_rss_kib,
            vm_size_kib: 0,
        };
        assert_eq!(rss_delta_kib(Some(sample(1_000)), Some(sample(1_250))), 250);
        assert_eq!(
            rss_delta_kib(Some(sample(1_250)), Some(sample(1_000))),
            -250
        );
        assert_eq!(rss_delta_kib(None, Some(sample(1_000))), 0);
        assert_eq!(rss_delta_kib(Some(sample(1_000)), None), 0);
    }
}
