//! 重型回測的全程序併發上限的測試。

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
