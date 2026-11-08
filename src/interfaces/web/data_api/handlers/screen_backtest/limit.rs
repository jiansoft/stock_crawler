//! 重型回測的全程序併發上限。
//!
//! 偏移、前緣與滾動前推一次請求就展開 20 組以上回測；單一請求內已用 `buffered(CONCURRENCY)`
//! 限制同時跑的組數，但多個請求同時進來時總量仍會疊加、占滿連線池。這裡再加一道全程序共用的
//! [`Semaphore`]：每組回測開跑前先取得許可。
//!
//! 取消傳播：`buffered` 是惰性的，呼叫端斷線或 Data API 逾時把回應 future 丟掉時，尚未開始的
//! 組數不會再被輪詢，也就不會再打資料庫；等待許可中的組數同樣隨之放棄。

use std::future::Future;
use std::time::{Duration, Instant};

use futures::{StreamExt, TryStreamExt, stream};
use tokio::sync::Semaphore;

use super::offsets::CONCURRENCY;

/// 全程序同時執行的重型回測組數（約為兩個請求各自的單請求上限）。
pub(super) const GLOBAL_PERMITS: usize = 2 * CONCURRENCY;

/// 等待許可超過這個時間就記錄，作為連線池壅塞的指標。
const SLOW_WAIT: Duration = Duration::from_millis(500);

static PERMITS: Semaphore = Semaphore::const_new(GLOBAL_PERMITS);

/// 依序對每個項目執行 `run`，同時最多 [`CONCURRENCY`] 組且受全程序上限約束；結果維持輸入順序，
/// 任一組失敗即停止並回傳該錯誤。
pub(super) async fn run_limited<I, T, E, F, Fut>(items: I, run: F) -> Result<Vec<T>, E>
where
    I: IntoIterator,
    F: Fn(I::Item) -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    run_limited_with(&PERMITS, CONCURRENCY, items, run).await
}

/// [`run_limited`] 的本體；許可來源與單請求併發數可注入，供測試驗證。
pub(super) async fn run_limited_with<I, T, E, F, Fut>(
    permits: &Semaphore,
    concurrency: usize,
    items: I,
    run: F,
) -> Result<Vec<T>, E>
where
    I: IntoIterator,
    F: Fn(I::Item) -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let run = &run;
    stream::iter(items)
        .map(|item| async move {
            let started = Instant::now();
            // 只有 `close()` 才會讓 acquire 失敗，這裡的 Semaphore 從不關閉。
            let _permit = permits
                .acquire()
                .await
                .expect("回測併發 semaphore 不會關閉");
            let waited = started.elapsed();
            if waited >= SLOW_WAIT {
                let waited_ms = waited.as_millis();
                tracing::info!(
                    "重型回測等待併發許可 {} ms（全程序上限 {GLOBAL_PERMITS}）",
                    waited_ms
                );
            }
            run(item).await
        })
        .buffered(concurrency)
        .try_collect()
        .await
}

#[cfg(test)]
mod tests;
