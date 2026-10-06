//! 依股票代號分組的追蹤條件快取，供價格更新事件與低頻對帳共用，
//! 避免每次價格變動都重新查詢整張 `trace` 資料表。

use std::collections::HashMap;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use once_cell::sync::Lazy;

use crate::{
    domain::trace::entity::PriceTrace, domain::trace::repository::TraceRepository,
    infra::database::repository::trace::PgTraceRepository,
};

/// 依股票代號分組後的追蹤條件快取。
static TRACE_TARGETS: Lazy<RwLock<HashMap<String, Vec<PriceTrace>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));
/// 標記追蹤條件快取是否至少成功載入過一次。
static TRACE_TARGETS_LOADED: AtomicBool = AtomicBool::new(false);

/// 追蹤條件快取的診斷快照。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::app::event::trace) struct TraceTargetDiagnostics {
    /// 目前被追蹤的股票代號數。
    pub symbol_count: usize,
    /// 追蹤條件總筆數。
    pub target_count: usize,
}

/// 以股票代號將追蹤條件分組。
fn group_targets_by_symbol(targets: Vec<PriceTrace>) -> HashMap<String, Vec<PriceTrace>> {
    let mut grouped_targets = HashMap::new();
    for target in targets {
        grouped_targets
            .entry(target.stock_symbol.clone())
            .or_insert_with(Vec::new)
            .push(target);
    }

    grouped_targets
}

/// 重新整理追蹤條件快取。
///
/// 此快取會依股票代號分組，供價格更新事件與低頻 reconciliation 共用，
/// 避免在每次價格變動時都重新查詢整張 `trace` 資料表。
pub(in crate::app::event::trace) async fn refresh_trace_targets_cache() -> Result<usize> {
    let trace_repo = PgTraceRepository::new();
    let targets = trace_repo.fetch_all().await?;
    let grouped_targets = group_targets_by_symbol(targets);
    let symbol_count = grouped_targets.len();

    if let Ok(mut cache) = TRACE_TARGETS.write() {
        *cache = grouped_targets;
    }

    TRACE_TARGETS_LOADED.store(true, Ordering::SeqCst);
    Ok(symbol_count)
}

/// 判斷追蹤條件快取是否至少成功載入過一次。
pub(in crate::app::event::trace) fn has_loaded_trace_targets_cache() -> bool {
    TRACE_TARGETS_LOADED.load(Ordering::SeqCst)
}

/// 取得追蹤條件快取的快照。
pub(super) fn get_grouped_targets_snapshot() -> HashMap<String, Vec<PriceTrace>> {
    TRACE_TARGETS
        .read()
        .map(|cache| cache.clone())
        .unwrap_or_default()
}

/// 取得指定股票代號的追蹤條件清單。
pub(super) fn get_targets_by_symbol(symbol: &str) -> Vec<PriceTrace> {
    TRACE_TARGETS
        .read()
        .ok()
        .and_then(|cache| cache.get(symbol).cloned())
        .unwrap_or_default()
}

/// 取得目前追蹤條件快取中的股票代號清單。
pub(in crate::app::event::trace) fn get_tracked_symbols() -> Vec<String> {
    let mut symbols = TRACE_TARGETS
        .read()
        .map(|cache| cache.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    symbols.sort();
    symbols
}

/// 判斷指定股票是否目前存在追蹤條件。
pub(in crate::app::event::trace) fn has_targets_for_symbol(symbol: &str) -> bool {
    TRACE_TARGETS
        .read()
        .map(|cache| cache.contains_key(symbol))
        .unwrap_or(false)
}

/// 取得追蹤條件快取目前的規模資訊。
pub(in crate::app::event::trace) fn trace_target_diagnostics() -> TraceTargetDiagnostics {
    TRACE_TARGETS
        .read()
        .map(|cache| TraceTargetDiagnostics {
            symbol_count: cache.len(),
            target_count: cache.values().map(Vec::len).sum(),
        })
        .unwrap_or_default()
}

/// 清空追蹤條件快取，供收盤後釋放記憶體使用。
pub(in crate::app::event::trace) fn clear_trace_targets_cache() {
    if let Ok(mut cache) = TRACE_TARGETS.write() {
        *cache = HashMap::new();
    }
    TRACE_TARGETS_LOADED.store(false, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
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
}
