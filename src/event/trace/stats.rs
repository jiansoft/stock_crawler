//! # Trace 執行統計模組
//!
//! 此模組負責累積股票追蹤流程在單次開盤期間的執行統計，
//! 供收盤或任務停止時輸出摘要，協助量化新版事件驅動追蹤的效果。
//!
//! 目前統計的指標包含：
//! - 已發佈的價格更新事件數
//! - 因 consumer 不存在或已停止而被丟棄的價格事件數
//! - 已被 consumer 實際處理的價格事件數
//! - 已送出的通知數
//! - reconciliation 執行次數
//! - reconciliation 掃描的股票數
//! - reconciliation 補救命中的通知數

use std::sync::atomic::{AtomicU64, Ordering};

use once_cell::sync::Lazy;

use crate::logging;

/// 單次追蹤執行期間的統計快照。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct TraceRuntimeStatsSnapshot {
    price_events_published: u64,
    price_events_dropped: u64,
    price_events_consumed: u64,
    notifications_sent: u64,
    reconciliation_runs: u64,
    reconciliation_symbols_scanned: u64,
    reconciliation_alert_hits: u64,
}

/// 追蹤流程使用的執行統計計數器集合。
#[derive(Default)]
struct TraceRuntimeStats {
    price_events_published: AtomicU64,
    price_events_dropped: AtomicU64,
    price_events_consumed: AtomicU64,
    notifications_sent: AtomicU64,
    reconciliation_runs: AtomicU64,
    reconciliation_symbols_scanned: AtomicU64,
    reconciliation_alert_hits: AtomicU64,
}

static TRACE_RUNTIME_STATS: Lazy<TraceRuntimeStats> = Lazy::new(TraceRuntimeStats::default);

fn format_rate(numerator: u64, denominator: u64) -> String {
    if denominator == 0 {
        return "0.00%".to_string();
    }

    format!("{:.2}%", numerator as f64 * 100.0 / denominator as f64)
}

fn take_runtime_stats_snapshot_and_reset() -> TraceRuntimeStatsSnapshot {
    TraceRuntimeStatsSnapshot {
        price_events_published: TRACE_RUNTIME_STATS
            .price_events_published
            .swap(0, Ordering::SeqCst),
        price_events_dropped: TRACE_RUNTIME_STATS
            .price_events_dropped
            .swap(0, Ordering::SeqCst),
        price_events_consumed: TRACE_RUNTIME_STATS
            .price_events_consumed
            .swap(0, Ordering::SeqCst),
        notifications_sent: TRACE_RUNTIME_STATS
            .notifications_sent
            .swap(0, Ordering::SeqCst),
        reconciliation_runs: TRACE_RUNTIME_STATS
            .reconciliation_runs
            .swap(0, Ordering::SeqCst),
        reconciliation_symbols_scanned: TRACE_RUNTIME_STATS
            .reconciliation_symbols_scanned
            .swap(0, Ordering::SeqCst),
        reconciliation_alert_hits: TRACE_RUNTIME_STATS
            .reconciliation_alert_hits
            .swap(0, Ordering::SeqCst),
    }
}

fn has_any_runtime_activity(snapshot: TraceRuntimeStatsSnapshot) -> bool {
    snapshot.price_events_published > 0
        || snapshot.price_events_dropped > 0
        || snapshot.price_events_consumed > 0
        || snapshot.notifications_sent > 0
        || snapshot.reconciliation_runs > 0
        || snapshot.reconciliation_symbols_scanned > 0
        || snapshot.reconciliation_alert_hits > 0
}

fn format_runtime_stats_summary(snapshot: TraceRuntimeStatsSnapshot) -> String {
    let dropped_rate = format_rate(
        snapshot.price_events_dropped,
        snapshot.price_events_published,
    );
    let consumed_rate = format_rate(
        snapshot.price_events_consumed,
        snapshot.price_events_published,
    );
    let reconciliation_hit_rate = format_rate(
        snapshot.reconciliation_alert_hits,
        snapshot.reconciliation_symbols_scanned,
    );

    format!(
        "Trace 收盤摘要 | 事件 發佈={:>6} 已處理={:>6}({:>7}) 丟棄={:>6}({:>7}) | 通知 已送出={:>6} | 對帳 次數={:>3} 掃描={:>6} 補救命中={:>6}({:>7})",
        snapshot.price_events_published,
        snapshot.price_events_consumed,
        consumed_rate,
        snapshot.price_events_dropped,
        dropped_rate,
        snapshot.notifications_sent,
        snapshot.reconciliation_runs,
        snapshot.reconciliation_symbols_scanned,
        snapshot.reconciliation_alert_hits,
        reconciliation_hit_rate,
    )
}

/// 重設追蹤執行統計。
pub(super) fn reset_runtime_stats() {
    let _ = take_runtime_stats_snapshot_and_reset();
}

/// 記錄一筆已發佈的價格更新事件。
pub(super) fn record_published_price_event() {
    TRACE_RUNTIME_STATS
        .price_events_published
        .fetch_add(1, Ordering::SeqCst);
}

/// 記錄一筆被丟棄的價格更新事件。
pub(super) fn record_dropped_price_event() {
    TRACE_RUNTIME_STATS
        .price_events_dropped
        .fetch_add(1, Ordering::SeqCst);
}

/// 記錄一筆已被 consumer 實際處理的價格更新事件。
pub(super) fn record_consumed_price_event() {
    TRACE_RUNTIME_STATS
        .price_events_consumed
        .fetch_add(1, Ordering::SeqCst);
}

/// 記錄一次已送出的追蹤通知。
pub(super) fn record_notification_sent() {
    TRACE_RUNTIME_STATS
        .notifications_sent
        .fetch_add(1, Ordering::SeqCst);
}

/// 記錄一次 reconciliation 執行，並累積本輪掃描股票數。
pub(super) fn record_reconciliation_run(symbol_count: usize) {
    TRACE_RUNTIME_STATS
        .reconciliation_runs
        .fetch_add(1, Ordering::SeqCst);
    TRACE_RUNTIME_STATS
        .reconciliation_symbols_scanned
        .fetch_add(symbol_count as u64, Ordering::SeqCst);
}

/// 記錄一次由 reconciliation 補救命中的通知。
pub(super) fn record_reconciliation_alert_hit() {
    TRACE_RUNTIME_STATS
        .reconciliation_alert_hits
        .fetch_add(1, Ordering::SeqCst);
}

/// 輸出單次追蹤執行期間的統計摘要，並在輸出後重設計數器。
///
/// 若本輪沒有任何活動，則不輸出摘要，避免收盤事件重複 stop 時留下全零 log。
pub(super) fn flush_runtime_stats() {
    let snapshot = take_runtime_stats_snapshot_and_reset();
    if !has_any_runtime_activity(snapshot) {
        return;
    }

    logging::info_file_async(format_runtime_stats_summary(snapshot));
}

#[cfg(test)]
mod tests {
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

        let snapshot = take_runtime_stats_snapshot_and_reset();
        assert_eq!(snapshot.price_events_published, 2);
        assert_eq!(snapshot.price_events_dropped, 1);
        assert_eq!(snapshot.price_events_consumed, 1);
        assert_eq!(snapshot.notifications_sent, 1);
        assert_eq!(snapshot.reconciliation_runs, 1);
        assert_eq!(snapshot.reconciliation_symbols_scanned, 3);
        assert_eq!(snapshot.reconciliation_alert_hits, 1);

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
        });

        assert_eq!(
            summary,
            "Trace 收盤摘要 | 事件 發佈=    10 已處理=     8( 80.00%) 丟棄=     2( 20.00%) | 通知 已送出=     3 | 對帳 次數=  2 掃描=    20 補救命中=     1(  5.00%)"
        );
    }
}
