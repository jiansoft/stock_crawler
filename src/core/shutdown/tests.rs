//! 應用程式平順關機所需的共用生命週期工具的測試。

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
