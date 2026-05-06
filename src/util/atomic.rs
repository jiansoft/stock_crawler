use std::sync::atomic::{AtomicUsize, Ordering};

/// 遞減原子計數器並傳回遞減後的新值。
///
/// 使用 `saturating_sub(1)` 確保不會低於 0。
pub fn decrement_atomic_usize(counter: &AtomicUsize) -> usize {
    counter
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
            Some(current.saturating_sub(1))
        })
        .map(|previous| previous.saturating_sub(1))
        .unwrap_or_default()
}
