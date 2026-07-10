//! 應用程式平順關機所需的共用生命週期工具。
//!
//! 此模組透過 watch channel 廣播關機訊號，並追蹤已開始且可能寫入資料的
//! 背景操作，讓主程式退出前能等待這些操作完成。

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use once_cell::sync::Lazy;
use tokio::sync::{Notify, watch};

/// 全域背景操作追蹤器，供 cron 與 manual backfill 共用。
pub static BACKGROUND_OPERATIONS: Lazy<BackgroundOperationTracker> =
    Lazy::new(BackgroundOperationTracker::new);

/// 追蹤可能具有資料副作用之背景操作的執行數量。
#[derive(Clone)]
pub struct BackgroundOperationTracker {
    /// 目前尚未完成的背景操作數量。
    active: Arc<AtomicUsize>,
    /// 操作完成時喚醒等待關機的主流程。
    idle_notify: Arc<Notify>,
}

impl BackgroundOperationTracker {
    /// 建立一個初始為 idle 的背景操作追蹤器。
    pub fn new() -> Self {
        Self {
            active: Arc::new(AtomicUsize::new(0)),
            idle_notify: Arc::new(Notify::new()),
        }
    }

    /// 登記一個已開始的背景操作，並回傳會在 drop 時自動結案的 guard。
    pub fn begin(&self) -> BackgroundOperationGuard {
        self.active.fetch_add(1, Ordering::AcqRel);
        // 狀態由 idle 轉為 active 時也喚醒等待者，讓關機流程重新計算安靜期。
        self.idle_notify.notify_one();
        BackgroundOperationGuard {
            tracker: self.clone(),
        }
    }

    /// 回傳目前尚未完成的背景操作數量，供關機 log 與健康檢查使用。
    pub fn active_count(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }

    /// 在指定時間內等待所有已登記的背景操作完成。
    ///
    /// 回傳 `true` 表示已進入 idle；`false` 表示等待逾時，呼叫端可記錄仍在執行的數量。
    pub async fn wait_for_idle(&self, timeout: Duration) -> bool {
        tokio::time::timeout(timeout, async {
            loop {
                if self.active_count() == 0 {
                    // 保留短暫安靜期，涵蓋 scheduler 已派發、但尚未第一次 poll 的 task。
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    if self.active_count() == 0 {
                        return;
                    }
                }

                self.idle_notify.notified().await;
            }
        })
        .await
        .is_ok()
    }
}

impl Default for BackgroundOperationTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// 背景操作的 RAII guard；離開成功、錯誤或 panic 路徑時都會自動減少 active counter。
pub struct BackgroundOperationGuard {
    /// 建立此 guard 的追蹤器。
    tracker: BackgroundOperationTracker,
}

impl Drop for BackgroundOperationGuard {
    fn drop(&mut self) {
        let previous = self.tracker.active.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "background operation counter underflow");

        // 只有最後一個操作完成時才需要喚醒關機等待者，減少不必要的排程喚醒。
        if previous == 1 {
            // `notify_one` 會在 waiter 尚未 poll 時保留 permit，避免完成通知遺失。
            self.tracker.idle_notify.notify_one();
        }
    }
}

/// 等待 watch channel 傳來關機訊號。
///
/// 若 sender 已先送出 `true`，函式會立即返回；若 sender 被意外 drop，也會返回，
/// 避免 server 因控制端消失而永遠無法停止。
pub async fn wait_for_shutdown(mut receiver: watch::Receiver<bool>) {
    if *receiver.borrow() {
        return;
    }

    let _ = receiver.wait_for(|shutdown| *shutdown).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 驗證 guard drop 後，idle waiter 會被喚醒。
    #[tokio::test]
    async fn tracker_waits_until_guard_is_dropped() {
        let tracker = BackgroundOperationTracker::new();
        let guard = tracker.begin();
        let waiter = {
            let tracker = tracker.clone();
            tokio::spawn(async move { tracker.wait_for_idle(Duration::from_secs(1)).await })
        };

        tokio::task::yield_now().await;
        assert_eq!(tracker.active_count(), 1);
        drop(guard);

        assert!(waiter.await.expect("idle waiter task should complete"));
        assert_eq!(tracker.active_count(), 0);
    }

    /// 驗證背景操作未結束時，等待會依指定時間停止。
    #[tokio::test]
    async fn tracker_reports_timeout_while_operation_is_active() {
        let tracker = BackgroundOperationTracker::new();
        let _guard = tracker.begin();

        assert!(!tracker.wait_for_idle(Duration::from_millis(10)).await);
    }

    /// 驗證已送出的關機訊號不會因接收端較晚開始等待而遺失。
    #[tokio::test]
    async fn shutdown_waiter_observes_existing_signal() {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        shutdown_tx
            .send(true)
            .expect("shutdown receiver should exist");

        wait_for_shutdown(shutdown_rx).await;
    }
}
