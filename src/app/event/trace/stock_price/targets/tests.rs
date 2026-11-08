//! 依股票代號分組的追蹤條件快取，供價格更新事件與低頻對帳共用，的測試。

use rust_decimal_macros::dec;

use super::*;

/// 驗證追蹤條件會依股票代號正確分組。
#[test]
fn test_group_targets_by_symbol() {
    let grouped = group_targets_by_symbol(vec![
        PriceTrace::new("2330".to_string(), dec!(500), dec!(600)),
        PriceTrace::new("2317".to_string(), dec!(100), dec!(120)),
        PriceTrace::new("2330".to_string(), dec!(520), dec!(650)),
    ]);

    assert_eq!(grouped.len(), 2);
    assert_eq!(grouped.get("2330").map(Vec::len), Some(2));
    assert_eq!(grouped.get("2317").map(Vec::len), Some(1));
}

#[test]
fn trace_targets_cache_snapshot_symbols_and_diagnostics_are_consistent() {
    let _lock = crate::infra::cache::REALTIME_TEST_LOCK.blocking_lock();
    clear_trace_targets_cache();
    assert!(!has_loaded_trace_targets_cache());
    assert_eq!(
        trace_target_diagnostics(),
        TraceTargetDiagnostics::default()
    );

    let grouped = group_targets_by_symbol(vec![
        PriceTrace::new("2330".to_string(), dec!(500), dec!(600)),
        PriceTrace::new("2317".to_string(), dec!(100), dec!(120)),
        PriceTrace::new("2330".to_string(), dec!(520), dec!(650)),
    ]);
    {
        let mut cache = TRACE_TARGETS.write().unwrap();
        *cache = grouped;
    }
    TRACE_TARGETS_LOADED.store(true, Ordering::SeqCst);

    assert!(has_loaded_trace_targets_cache());
    assert_eq!(get_tracked_symbols(), vec!["2317", "2330"]);
    assert!(has_targets_for_symbol("2330"));
    assert!(!has_targets_for_symbol("0050"));
    assert_eq!(get_targets_by_symbol("2330").len(), 2);
    assert_eq!(
        trace_target_diagnostics(),
        TraceTargetDiagnostics {
            symbol_count: 2,
            target_count: 3,
        }
    );

    clear_trace_targets_cache();
    assert!(!has_loaded_trace_targets_cache());
    assert!(get_tracked_symbols().is_empty());
}
