use std::sync::atomic::{AtomicUsize, Ordering};

/// 遞減原子計數器並傳回遞減後的新值。
///
/// 使用 `saturating_sub(1)` 確保不會低於 0。
pub fn decrement_atomic_usize(counter: &AtomicUsize) -> usize {
    let mut current = counter.load(Ordering::SeqCst);
    loop {
        let new_val = current.saturating_sub(1);
        match counter.compare_exchange_weak(current, new_val, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return new_val,
            Err(v) => current = v,
        }
    }
}
