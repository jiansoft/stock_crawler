//! # Yahoo 股利政策頁的重試抓取
//!
//! 股利回補流程共用的 Yahoo 抓取入口，統一重試策略。
//!
//! ## 為什麼不能用 `ExponentialBackoff::from_millis(100)`
//!
//! tokio-retry 的 `ExponentialBackoff` 延遲是 `base^n` 毫秒，而不是 `base × 2^n`：
//! `from_millis(100)` 依序等 100ms、10s、1000s、100000s（27.8 小時）、10⁷s（115 天）。
//! 2026-09-21 的 21:00 股利排程就因此卡住：4987 回 404 後照樣重試，
//! 第四次重試在 24.75 小時後才發出，第五次得等到數十天後，整個 `execute()` 永遠不結束，
//! 還讓服務關機時的背景作業等待逾時。
//!
//! 現行策略：延遲 `500ms × 2^n`（上限 5 秒）再乘上 0～1 的隨機抖動，最多重試 3 次；
//! 404（頁面不存在，通常是已下市）重試也不會變好，直接失敗。
//! 網路層的逾時與重試另由 `core::util::http` 處理，這裡只是外層的保底。

use std::time::Duration;

use anyhow::Result;
use tokio_retry::{
    RetryIf,
    strategy::{ExponentialBackoff, jitter},
};

use crate::infra::crawler::yahoo::{self, dividend::YahooDividend};

/// 最多重試次數（不含第一次請求）。
const MAX_RETRIES: usize = 3;

/// 單次重試延遲上限（套用抖動前）。
const MAX_RETRY_DELAY: Duration = Duration::from_secs(5);

/// 建立重試延遲序列（未套用抖動）：500ms、1s、2s，單次不超過 [`MAX_RETRY_DELAY`]。
fn retry_delays() -> impl Iterator<Item = Duration> {
    ExponentialBackoff::from_millis(2)
        .factor(250)
        .max_delay(MAX_RETRY_DELAY)
        .take(MAX_RETRIES)
}

/// 抓取單一股票的 Yahoo 股利政策頁，暫時性錯誤最多重試 [`MAX_RETRIES`] 次。
///
/// # Errors
///
/// 重試用盡或頁面不存在（404，可用 [`yahoo::dividend::is_page_not_found_error`] 辨識）時回傳最後一次的錯誤。
pub(super) async fn visit_with_retry(stock_symbol: &str) -> Result<YahooDividend> {
    RetryIf::start(
        retry_delays().map(jitter),
        || yahoo::dividend::visit(stock_symbol),
        |err: &anyhow::Error| !yahoo::dividend::is_page_not_found_error(err),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 重試延遲必須是秒級且有上限，總等待時間不可失控。
    #[test]
    fn retry_delays_are_bounded() {
        let delays: Vec<Duration> = retry_delays().collect();

        assert_eq!(
            delays,
            vec![
                Duration::from_millis(500),
                Duration::from_secs(1),
                Duration::from_secs(2),
            ]
        );
        assert!(delays.iter().all(|delay| *delay <= MAX_RETRY_DELAY));
    }

    /// 已下市的 4987 回 404，必須立即失敗而不是進入重試。
    #[tokio::test]
    #[ignore]
    async fn visit_with_retry_does_not_retry_page_not_found() {
        dotenvy::dotenv().ok();

        let started = std::time::Instant::now();
        let err = visit_with_retry("4987").await.expect_err("4987 應回 404");

        assert!(yahoo::dividend::is_page_not_found_error(&err));
        assert!(started.elapsed() < Duration::from_millis(400));
    }
}
