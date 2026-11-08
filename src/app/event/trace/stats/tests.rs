//! Trace 執行統計模組的測試。

use super::*;

/// 驗證統計快照會正確累積資料，並在取出後重設為零。
#[test]
fn test_take_runtime_stats_snapshot_and_reset() {
    reset_runtime_stats();

    record_published_price_event();
    record_published_price_event();
    record_dropped_price_event();
    record_consumed_price_event();
    record_notification_sent();
    record_reconciliation_run(3);
    record_reconciliation_alert_hit();
    record_redis_dedup_failure();

    let snapshot = take_runtime_stats_snapshot_and_reset();
    assert_eq!(snapshot.price_events_published, 2);
    assert_eq!(snapshot.price_events_dropped, 1);
    assert_eq!(snapshot.price_events_consumed, 1);
    assert_eq!(snapshot.notifications_sent, 1);
    assert_eq!(snapshot.reconciliation_runs, 1);
    assert_eq!(snapshot.reconciliation_symbols_scanned, 3);
    assert_eq!(snapshot.reconciliation_alert_hits, 1);
    assert_eq!(snapshot.redis_dedup_failures, 1);

    let reset_snapshot = take_runtime_stats_snapshot_and_reset();
    assert_eq!(reset_snapshot, TraceRuntimeStatsSnapshot::default());
}

/// 驗證收盤摘要格式會輸出單行且包含主要比率資訊。
#[test]
fn test_format_runtime_stats_summary() {
    let summary = format_runtime_stats_summary(TraceRuntimeStatsSnapshot {
        price_events_published: 10,
        price_events_dropped: 2,
        price_events_consumed: 8,
        notifications_sent: 3,
        reconciliation_runs: 2,
        reconciliation_symbols_scanned: 20,
        reconciliation_alert_hits: 1,
        redis_dedup_failures: 4,
    });

    assert_eq!(
        summary,
        "Trace 收盤摘要 | 事件 發佈=    10 已處理=     8( 80.00%) 丟棄=     2( 20.00%) | 通知 已送出=     3 | 對帳 次數=  2 掃描=    20 補救命中=     1(  5.00%) | Redis 去重失敗=     4"
    );
}

#[test]
fn format_rate_returns_zero_when_denominator_is_zero() {
    assert_eq!(format_rate(10, 0), "0.00%");
    assert_eq!(format_rate(0, 10), "0.00%");
    assert_eq!(format_rate(1, 4), "25.00%");
}

#[test]
fn has_any_runtime_activity_detects_each_counter() {
    assert!(!has_any_runtime_activity(
        TraceRuntimeStatsSnapshot::default()
    ));

    assert!(has_any_runtime_activity(TraceRuntimeStatsSnapshot {
        price_events_published: 1,
        ..Default::default()
    }));

    assert!(has_any_runtime_activity(TraceRuntimeStatsSnapshot {
        price_events_dropped: 1,
        ..Default::default()
    }));

    assert!(has_any_runtime_activity(TraceRuntimeStatsSnapshot {
        price_events_consumed: 1,
        ..Default::default()
    }));

    assert!(has_any_runtime_activity(TraceRuntimeStatsSnapshot {
        notifications_sent: 1,
        ..Default::default()
    }));

    assert!(has_any_runtime_activity(TraceRuntimeStatsSnapshot {
        reconciliation_runs: 1,
        ..Default::default()
    }));

    assert!(has_any_runtime_activity(TraceRuntimeStatsSnapshot {
        reconciliation_symbols_scanned: 1,
        ..Default::default()
    }));

    assert!(has_any_runtime_activity(TraceRuntimeStatsSnapshot {
        reconciliation_alert_hits: 1,
        ..Default::default()
    }));

    assert!(has_any_runtime_activity(TraceRuntimeStatsSnapshot {
        redis_dedup_failures: 1,
        ..Default::default()
    }));
}

#[test]
fn current_runtime_stats_snapshot_does_not_reset_counters() {
    reset_runtime_stats();
    record_published_price_event();
    record_reconciliation_run(2);

    let snapshot = get_runtime_stats_snapshot();
    assert_eq!(snapshot.price_events_published, 1);
    assert_eq!(snapshot.reconciliation_runs, 1);
    assert_eq!(snapshot.reconciliation_symbols_scanned, 2);

    let second_snapshot = get_runtime_stats_snapshot();
    assert_eq!(second_snapshot, snapshot);

    reset_runtime_stats();
}
