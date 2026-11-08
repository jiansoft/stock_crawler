//! Yahoo 類股快取背景任務的生命週期旗標與最近一輪診斷數據的測試。

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
