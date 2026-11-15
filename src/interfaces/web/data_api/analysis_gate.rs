//! 重運算（回測）的全域並行號誌（會員系統計畫 2.3「重運算租約規格」③）。
//!
//! 外部請求（Data API 的 `/backtest` 端點）與內部排程（策略換股、策略優勢檢查直接呼叫回測引擎）
//! 共用同一組名額，避免多個帳號或多個 MCP Token 一起耗盡 pi 的資源：
//! - 外部請求：名額滿時最多 [`AnalysisGate::queue_max`] 個可以排隊等候，其餘立刻回 429；
//!   等候時間算在請求上限內。
//! - 內部排程：一律等候，不被拒絕。

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// 全服務同時執行的重運算上限（環境變數）。
pub(crate) const MAX_CONCURRENT_ENV: &str = "ANALYSIS_MAX_CONCURRENT_GLOBAL";
/// 外部請求可排隊等候名額的上限（環境變數）；未設定視為 0（滿載立刻拒絕）。
pub(crate) const QUEUE_MAX_ENV: &str = "ANALYSIS_QUEUE_MAX";
/// 未設定 [`MAX_CONCURRENT_ENV`] 時的名額。計畫要求不得以無上限預設值開放，正式值在 L8 於 pi 實測後填入；
/// pi 為 4 核心，回測以資料庫查詢為主，先保守取 2。
pub(crate) const DEFAULT_MAX_CONCURRENT: usize = 2;

/// 全服務共用的重運算名額。
pub(crate) static GATE: Lazy<Arc<AnalysisGate>> = Lazy::new(|| Arc::new(AnalysisGate::from_env()));

/// 重運算並行號誌。
#[derive(Debug)]
pub(crate) struct AnalysisGate {
    semaphore: Arc<Semaphore>,
    queue_max: usize,
    waiting: AtomicUsize,
}

/// 等候中的外部請求計數；離開等候（取得名額、逾時或請求被丟棄）時自動減一。
struct WaitingGuard<'a>(&'a AtomicUsize);

impl Drop for WaitingGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl AnalysisGate {
    /// 以名額數與排隊上限建立號誌；名額至少 1。
    pub(crate) fn new(max_concurrent: usize, queue_max: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent.max(1))),
            queue_max,
            waiting: AtomicUsize::new(0),
        }
    }

    /// 由環境變數建立；未設定或無法解析時使用預設值。
    pub(crate) fn from_env() -> Self {
        let read = |name: &str| {
            std::env::var(name)
                .ok()
                .and_then(|value| value.trim().parse::<usize>().ok())
        };
        Self::new(
            read(MAX_CONCURRENT_ENV).unwrap_or(DEFAULT_MAX_CONCURRENT),
            read(QUEUE_MAX_ENV).unwrap_or(0),
        )
    }

    /// 外部請求取得名額：有空位立即取得；滿載時排隊人數未達上限就等候（最多 `wait_limit`），
    /// 否則回 `None`（呼叫端回 429）。
    pub(crate) async fn acquire_external(
        &self,
        wait_limit: Duration,
    ) -> Option<OwnedSemaphorePermit> {
        if let Ok(permit) = Arc::clone(&self.semaphore).try_acquire_owned() {
            return Some(permit);
        }
        // 先佔排隊位置再檢查，避免兩個請求同時通過「未達上限」的判斷
        if self.waiting.fetch_add(1, Ordering::AcqRel) >= self.queue_max {
            self.waiting.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        let _waiting = WaitingGuard(&self.waiting);
        tokio::time::timeout(wait_limit, Arc::clone(&self.semaphore).acquire_owned())
            .await
            .ok()
            .and_then(Result::ok)
    }

    /// 內部排程取得名額：一律等候。
    ///
    /// # Errors
    ///
    /// 號誌被關閉時回錯誤（本模組從不關閉號誌，正常不會發生）。
    pub(crate) async fn acquire_internal(&self) -> Result<OwnedSemaphorePermit> {
        Arc::clone(&self.semaphore)
            .acquire_owned()
            .await
            .context("重運算名額號誌已關閉")
    }

    /// 目前可用的名額數（測試用）。
    #[cfg(test)]
    pub(crate) fn available(&self) -> usize {
        self.semaphore.available_permits()
    }
}

#[cfg(test)]
mod tests;
