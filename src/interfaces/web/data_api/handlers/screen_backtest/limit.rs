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
                tracing::info!(
                    "重型回測等待併發許可 {} ms（全程序上限 {GLOBAL_PERMITS}）",
                    waited.as_millis()
                );
            }
            run(item).await
        })
        .buffered(concurrency)
        .try_collect()
        .await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// 同時執行數不超過全程序許可數，即使單請求併發數較大；結果維持輸入順序。
    #[tokio::test]
    async fn respects_global_permits_and_keeps_order() {
        let permits = Semaphore::new(2);
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let result: Result<Vec<u32>, ()> = run_limited_with(&permits, 8, 0..10u32, |n| {
            let (running, peak) = (running.clone(), peak.clone());
            async move {
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(5)).await;
                running.fetch_sub(1, Ordering::SeqCst);
                Ok(n * 2)
            }
        })
        .await;
        assert_eq!(result.unwrap(), (0..10).map(|n| n * 2).collect::<Vec<_>>());
        assert!(peak.load(Ordering::SeqCst) <= 2, "peak={peak:?}");
        assert_eq!(permits.available_permits(), 2, "許可全部歸還");
    }

    /// 呼叫端丟棄 future（斷線或逾時）後，尚未開始的組數不會再執行，許可全部歸還。
    #[tokio::test]
    async fn dropping_the_request_stops_pending_runs() {
        let permits = Semaphore::new(4);
        let started = Arc::new(AtomicUsize::new(0));
        let work = run_limited_with(&permits, 2, 0..20u32, |_| {
            let started = started.clone();
            async move {
                started.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok::<_, ()>(())
            }
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(30), work)
                .await
                .is_err()
        );
        assert_eq!(
            started.load(Ordering::SeqCst),
            2,
            "只有單請求併發數內的組數開跑"
        );
        assert_eq!(permits.available_permits(), 4);
    }

    /// 任一組失敗即回傳錯誤，後續組數不再開跑。
    #[tokio::test]
    async fn first_error_short_circuits() {
        let permits = Semaphore::new(4);
        let started = Arc::new(AtomicUsize::new(0));
        let result = run_limited_with(&permits, 1, 0..10u32, |n| {
            let started = started.clone();
            async move {
                started.fetch_add(1, Ordering::SeqCst);
                if n == 2 { Err(n) } else { Ok(n) }
            }
        })
        .await;
        assert_eq!(result, Err(2));
        assert_eq!(started.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn slow_wait_is_logged_without_failing() {
        let permits = Semaphore::new(1);
        let held = permits.acquire().await.unwrap();
        let work = run_limited_with(&permits, 1, [1u32], |n| async move { Ok::<_, ()>(n) });
        let release = async {
            tokio::time::sleep(SLOW_WAIT + Duration::from_millis(20)).await;
            drop(held);
        };
        let (result, ()) = tokio::join!(work, release);
        assert_eq!(result, Ok(vec![1]));
    }
}
