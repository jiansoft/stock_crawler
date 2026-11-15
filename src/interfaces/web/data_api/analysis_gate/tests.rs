//! 重運算全域號誌的單元測試。

use std::time::Duration;

use super::*;

/// 滿載且不允許排隊時立刻拒絕；名額歸還後又可取得。
#[tokio::test]
async fn external_is_rejected_when_full_without_queue() {
    let gate = AnalysisGate::new(1, 0);
    let held = gate
        .acquire_external(Duration::from_millis(10))
        .await
        .expect("第一個請求取得名額");
    assert_eq!(gate.available(), 0);
    assert!(
        gate.acquire_external(Duration::from_secs(5))
            .await
            .is_none()
    );
    drop(held);
    assert!(
        gate.acquire_external(Duration::from_millis(10))
            .await
            .is_some()
    );
}

/// 允許排隊時等到名額歸還；等候超過上限則放棄；排隊人數有上限。
#[tokio::test]
async fn external_waits_within_queue_limit() {
    let gate = Arc::new(AnalysisGate::new(1, 1));
    let held = gate
        .acquire_external(Duration::from_millis(10))
        .await
        .expect("取得名額");
    // 等候逾時
    assert!(
        gate.acquire_external(Duration::from_millis(20))
            .await
            .is_none()
    );
    // 一個在排隊時，第二個排隊者被拒
    let waiter = {
        let gate = Arc::clone(&gate);
        tokio::spawn(async move {
            gate.acquire_external(Duration::from_secs(5))
                .await
                .is_some()
        })
    };
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(
        gate.acquire_external(Duration::from_secs(5))
            .await
            .is_none()
    );
    drop(held);
    assert!(waiter.await.expect("等候工作"));
}

/// 內部排程一律等候，不被拒絕。
#[tokio::test]
async fn internal_always_waits() {
    let gate = Arc::new(AnalysisGate::new(1, 0));
    let held = gate.acquire_internal().await.expect("取得名額");
    let internal = {
        let gate = Arc::clone(&gate);
        tokio::spawn(async move { gate.acquire_internal().await.is_ok() })
    };
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!internal.is_finished());
    drop(held);
    assert!(internal.await.expect("內部工作"));
}

/// 名額至少 1；未設定環境變數時使用預設值。
#[test]
fn capacity_is_at_least_one() {
    assert_eq!(AnalysisGate::new(0, 0).available(), 1);
    assert_eq!(AnalysisGate::new(3, 0).available(), 3);
}
