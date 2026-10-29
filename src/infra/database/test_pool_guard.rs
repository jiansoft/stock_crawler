//! 測試專用：偵測 tokio runtime「世代」交替，讓連線池能辨識殭屍連線。
//!
//! # 背景（為什麼需要這個模組）
//! 測試組建的連線池是 process 級單例，但每個 `#[tokio::test]` 都有自己的
//! tokio runtime。連線的 socket readiness 註冊在「建立它的 runtime」的
//! IO driver 上；runtime 結束後這些通知永遠不會再送達，任何在這種
//! 「殭屍連線」上等待回應的操作（查詢、ping、graceful close）都會卡死。
//!
//! # 原理
//! 我們無法從 sqlx 的 hook 直接得知「這條連線屬於哪個 runtime」
//! （`PgConnection` 沒有暴露這種身分），但可以用時間軸推斷：
//! 1. runtime 上第一次「建立新連線」（after_connect）或「借出閒置連線」
//!    （before_acquire）時，種下一個「哨兵 task」（永不結束）。
//!    兩個 hook 都要種：只靠 before_acquire 的話，「整個測試只用全新連線」
//!    的 runtime 死亡時就沒有哨兵可 drop，世代起點不會重設，
//!    它留下的年輕殭屍連線會被下一個測試誤判為本世代而重用（實測會卡死）。
//! 2. runtime 關閉時 tokio 會 drop 所有未完成的 task——哨兵的 Drop
//!    把全域的「世代起點」重設為當下時刻。
//! 3. 借出連線時比較「連線年齡」與「世代起點」：
//!    連線比世代起點更老＝誕生於上一個 runtime → 是殭屍，必須丟棄。
//!
//! 測試以 `--test-threads=1` 循序執行，任一時刻只有一個 runtime 存在，
//! 所以單一全域世代起點就足夠了。

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;

/// 目前 runtime 世代的起點時刻；哨兵被 drop（runtime 關閉）時重設。
static EPOCH_START: Lazy<Mutex<Instant>> = Lazy::new(|| Mutex::new(Instant::now()));
/// 目前世代是否已種下哨兵（每個 runtime 只需要一個）。
static SENTINEL_ALIVE: AtomicBool = AtomicBool::new(false);

/// 哨兵守衛：`Drop` 代表 runtime 正在關閉 → 開啟新世代。
struct SentinelGuard;

impl Drop for SentinelGuard {
    fn drop(&mut self) {
        if let Ok(mut epoch) = EPOCH_START.lock() {
            *epoch = Instant::now();
        }
        SENTINEL_ALIVE.store(false, Ordering::SeqCst);
    }
}

/// 確保目前 runtime 上有一個存活的哨兵 task。
///
/// 以 `swap` 原子地占用種植權，避免同一 runtime 種出多個哨兵。
/// 哨兵本體是一個掛在 `pending()` 上的 task——它唯一的工作
/// 就是「在 runtime 關閉時被 drop」，讓 [`SentinelGuard`] 重設世代。
pub(super) fn ensure_runtime_sentinel() {
    if !SENTINEL_ALIVE.swap(true, Ordering::SeqCst) {
        tokio::spawn(async {
            let _guard = SentinelGuard;
            std::future::pending::<()>().await;
        });
    }
}

/// 判斷一條閒置連線是否誕生於上一個 runtime 世代。
///
/// `age` 來自 sqlx 的 `PoolConnectionMetadata::age`（距離建線的時間）。
/// 若連線比目前世代起點更老，代表它是上一個 runtime 留下的殭屍連線。
pub(super) fn is_stale(age: Duration) -> bool {
    let epoch_elapsed = EPOCH_START
        .lock()
        .map(|epoch| epoch.elapsed())
        // lock 失敗（理論上不會發生）時當作「世代剛開始」，
        // 讓所有閒置連線都被丟棄——寧可多重連，不可冒卡死風險。
        .unwrap_or(Duration::ZERO);

    age > epoch_elapsed
}
