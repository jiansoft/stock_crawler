//! 價格追蹤診斷的單元測試。

use super::*;

/// 直接跑一輪診斷：事件計數的差值要寫回 previous_stats，計時點要往前推。
#[test]
fn log_trace_diagnostics_advances_the_baseline() {
    let mut previous_stats = trace_stats::TraceRuntimeStatsSnapshot::default();
    let started_at = Instant::now() - Duration::from_secs(30);
    let mut previous_logged_at = started_at;
    // 剛整理過記憶體，這一輪不應再觸發整理。
    let mut previous_trimmed_at = Instant::now();
    let trimmed_at = previous_trimmed_at;

    trace_stats::record_published_price_event();
    log_trace_diagnostics(
        &mut previous_stats,
        &mut previous_logged_at,
        &mut previous_trimmed_at,
    );

    // 其他測試也可能同時累加全域計數器，只確認基準已更新到包含剛才那一筆。
    assert!(previous_stats.price_events_published >= 1);
    assert!(previous_logged_at > started_at);
    assert_eq!(previous_trimmed_at, trimmed_at);
}

/// 還有待處理事件、日誌佇列未清空或距上次整理太近時，都不可整理記憶體。
#[test]
fn maybe_trim_allocator_waits_for_an_idle_snapshot() {
    let long_ago = Instant::now() - TRACE_ALLOCATOR_TRIM_INTERVAL - Duration::from_secs(1);
    let cases = [(1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1)];
    for (pending, backlog, default_queued, http_queued) in cases {
        let mut trimmed_at = long_ago;
        maybe_trim_allocator(
            &mut trimmed_at,
            pending,
            backlog,
            default_queued,
            http_queued,
        );
        assert_eq!(trimmed_at, long_ago, "忙碌時不應更新整理時間");
    }

    let recent = Instant::now();
    let mut trimmed_at = recent;
    maybe_trim_allocator(&mut trimmed_at, 0, 0, 0, 0);
    assert_eq!(trimmed_at, recent, "未滿整理間隔不應更新整理時間");

    let mut trimmed_at = long_ago;
    maybe_trim_allocator(&mut trimmed_at, 0, 0, 0, 0);
    assert!(trimmed_at > long_ago, "閒置且超過間隔時要記錄這次整理");
}

#[test]
fn snapshot_cache_diagnostics_reports_reserved_bytes() {
    let (len, capacity, string_bytes, reserved_bytes) = snapshot_cache_diagnostics();
    assert!(capacity >= len);
    assert!(reserved_bytes >= string_bytes);
}

#[test]
fn task_status_helpers_read_the_counters() {
    let active = AtomicUsize::new(2);
    let generation = AtomicU64::new(7);
    let status = atomic_task_status(true, &active, &generation);
    assert_eq!(status, TaskRuntimeStatus::new(true, 2, 7));
    assert_eq!(
        format_task_status("backup", status),
        "backup(en=true active=2 gen=7)"
    );

    // consumer 未啟動時 sender 不存在，狀態應為停用。
    if PRICE_UPDATE_TX
        .read()
        .map(|tx| tx.is_none())
        .unwrap_or(false)
    {
        assert!(!price_consumer_status().enabled);
    }
}

#[test]
fn memory_formatting_helpers() {
    assert_eq!(kib_to_mib(2048), 2.0);
    assert_eq!(format_signed_kib(12), "+12");
    assert_eq!(format_signed_kib(0), "+0");
    assert_eq!(format_signed_kib(-5), "-5");
}

/// 診斷任務可以啟動、跑過至少一輪，並在停止通知後結束。
#[tokio::test(start_paused = true)]
async fn diagnostics_task_starts_logs_and_stops() {
    start_trace_diagnostics_task();
    // 虛擬時鐘：跨過兩個診斷週期，讓迴圈實際跑過 log_trace_diagnostics。
    tokio::time::sleep(TRACE_DIAGNOSTICS_LOG_INTERVAL * 2 + Duration::from_secs(1)).await;
    assert!(DIAGNOSTICS_LAST_GENERATION.load(Ordering::SeqCst) >= 1);

    stop_trace_diagnostics_task();
    for _ in 0..100 {
        TRACE_TASK_STOP_NOTIFY.notify_waiters();
        tokio::time::sleep(Duration::from_millis(50)).await;
        if DIAGNOSTICS_ACTIVE_TASKS.load(Ordering::SeqCst) == 0
            && !IS_DIAGNOSTICS_LOGGING.load(Ordering::SeqCst)
        {
            return;
        }
    }
    panic!("診斷任務未在期限內停止");
}
