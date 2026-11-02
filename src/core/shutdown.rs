//! 應用程式平順關機所需的共用生命週期工具。
//!
//! 此模組透過 watch channel 廣播關機訊號，並追蹤已開始且可能寫入資料的
//! 背景操作，讓主程式退出前能等待這些操作完成。
//!
//! ## 新進開發者先看這裡
//!
//! 本專案有兩種需要分開處理的非同步工作：
//!
//! 1. **Server accept loop**：Axum 與 Tonic 長時間等待新連線。它們透過
//!    [`watch`] channel 接收「準備關機」狀態，收到後停止接受新連線，但會讓已進入
//!    handler 的 request 繼續完成。
//! 2. **資料背景操作**：cron 與 manual backfill 可能正在寫 PostgreSQL。這些操作不能
//!    在任意 `.await` 點直接 abort，否則可能只完成一半。因此用
//!    [`BackgroundOperationTracker`] 計數，讓 `main` 等到 active count 歸零。
//!
//! 可以把整體流程想成：
//!
//! ```text
//! OS signal
//!    │
//!    ├─ watch=true ──> HTTP/gRPC 停止收新 request ──> 排空既有 request
//!    │
//!    ├─ scheduler.shutdown() ──> 不再建立新的 cron 操作
//!    │
//!    └─ wait_for_idle() ──> 等既有 cron/backfill guard 全部 drop
//! ```
//!
//! watch channel 在這裡比一次性 event 更合適，因為它保存「目前是否已關機」的狀態；
//! 即使 receiver 比 sender 晚開始等待，也仍會看到已經送出的 `true`。

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use once_cell::sync::Lazy;
use tokio::sync::{Notify, watch};

/// 全域背景操作追蹤器，供 cron 與 manual backfill 共用。
///
/// 只有「任意中斷可能留下半成品」的工作需要登記。單純的 diagnostics 或可隨時重建的
/// memory cache 不必放進來，否則無關緊要的 task 也會拖長正式關機時間。
pub static BACKGROUND_OPERATIONS: Lazy<BackgroundOperationTracker> =
    Lazy::new(BackgroundOperationTracker::new);

/// 全域關機請求，供逐檔查外站、一跑數十分鐘的排程迴圈在每檔之間檢查。
///
/// [`BACKGROUND_OPERATIONS`] 只能等工作自己結束：2026-11-02 21:08 部署時，21:00 股利補抓
/// （平常跑約 30 分鐘）不理會關機訊號，主程式等滿五分鐘才硬停，當輪補抓也中斷在半途。
/// 迴圈看到請求就停在兩檔之間，已處理的都已寫入，沒處理的不寫跳過旗標，下一輪自然接續。
pub static SHUTDOWN_REQUEST: ShutdownRequest = ShutdownRequest::new();

/// 「已要求關機」旗標；只會從 `false` 變成 `true`，不會再變回來。
#[derive(Debug, Default)]
pub struct ShutdownRequest {
    /// 是否已收到關機訊號。
    requested: AtomicBool,
}

impl ShutdownRequest {
    /// 建立尚未要求關機的旗標。
    pub const fn new() -> Self {
        Self {
            requested: AtomicBool::new(false),
        }
    }

    /// 標記已收到關機訊號；`bootstrap::shutdown` 在通知 server 之前呼叫。
    pub fn request(&self) {
        self.requested.store(true, Ordering::Release);
    }

    /// 已收到關機訊號時回傳 `true`。
    pub fn is_requested(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }
}

/// 追蹤可能具有資料副作用之背景操作的執行數量。
///
/// 此型別刻意只保存 counter 與通知器，不保存每個 task 的 [`tokio::task::JoinHandle`]。
/// 原因是 cron task 由第三方 scheduler 建立，而 manual backfill 又由 request handler 建立，
/// 兩者沒有共同擁有 JoinHandle 的地方；RAII counter 可以用同一種方式涵蓋兩條路徑。
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
    ///
    /// 呼叫端應在 spawn 前或 operation future 一開始就取得 guard，並把 guard 綁在
    /// operation 的作用域。不要手動呼叫「完成」函式；Rust 的 drop 才能同時涵蓋
    /// `Ok`、`Err`、提早 `return` 與 panic unwind。
    pub fn begin(&self) -> BackgroundOperationGuard {
        self.active.fetch_add(1, Ordering::AcqRel);
        // 狀態由 idle 轉為 active 時也喚醒等待者，讓關機流程重新計算安靜期。
        self.idle_notify.notify_one();
        BackgroundOperationGuard {
            tracker: self.clone(),
        }
    }

    /// 回傳目前尚未完成的背景操作數量，供關機 log 與健康檢查使用。
    ///
    /// 這個數字只用於觀測與判斷是否 idle，不代表 queue 長度，也不應拿來做業務限流。
    pub fn active_count(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }

    /// 在指定時間內等待所有已登記的背景操作完成。
    ///
    /// 回傳 `true` 表示已進入 idle；`false` 表示等待逾時，呼叫端可記錄仍在執行的數量。
    ///
    /// 這裡保留 100ms「安靜期」，是因為 scheduler 可能已把 future 排進 Tokio queue，
    /// 但該 future 尚未第一次 poll、也就尚未執行 [`Self::begin`]。若看到一次零就立刻返回，
    /// 主程式可能在那個 future 真正開始前先 drop runtime。
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
///
/// 對初學者而言，可以把它理解成「借來的一張工作證」：取得時 active `+1`，工作 scope
/// 結束、工作證被 drop 時 active `-1`。主程式只在所有工作證都歸還後才安全退出。
pub struct BackgroundOperationGuard {
    /// 建立此 guard 的追蹤器。
    tracker: BackgroundOperationTracker,
}

impl Drop for BackgroundOperationGuard {
    fn drop(&mut self) {
        // fetch_sub 回傳的是減一前的值，因此 previous == 1 代表本 guard 是最後一個操作。
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
///
/// 這個函式會交給 Axum/Tonic 的 graceful shutdown API 當作停止 future；它本身不 abort
/// request，也不負責等待背景資料工作，兩者分別由 server 與 tracker 處理。
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

    /// 關機請求只會由未要求變成已要求。
    #[test]
    fn shutdown_request_flips_once_requested() {
        let request = ShutdownRequest::default();
        assert!(!request.is_requested());

        request.request();
        request.request();
        assert!(request.is_requested());
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
