//! trace 即時價格任務協調的測試：追蹤代號、價格更新佇列、背景任務啟停與快取就緒等待。

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tokio::sync::mpsc;

use super::*;
use crate::domain::trace::entity::PriceTrace;

/// 將追蹤設定整理成不重複的股票代號清單。
fn collect_traced_symbols(targets: Vec<PriceTrace>) -> Vec<String> {
    let mut symbols = targets
        .into_iter()
        .map(|target| target.stock_symbol)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    symbols.sort();
    symbols
}

/// 驗證追蹤股票代號整理流程會去除重複值並依字串排序。
#[test]
fn test_collect_traced_symbols_deduplicates_and_sorts() {
    let symbols = collect_traced_symbols(vec![
        PriceTrace::new("2330".to_string(), dec!(500), dec!(600)),
        PriceTrace::new("2317".to_string(), dec!(100), dec!(120)),
        PriceTrace::new("2330".to_string(), dec!(520), dec!(650)),
    ]);

    assert_eq!(symbols, vec!["2317".to_string(), "2330".to_string()]);
}

/// 驗證在未啟動 consumer 時發佈價格事件不會 panic。
#[test]
fn test_publish_price_update_without_consumer() {
    publish_price_update("2330".to_string(), dec!(998));
}
/// 這組測試都會改動全域狀態（sender、pending、旗標、追蹤條件快取、即時報價快取），
/// 與其他盤中報價測試共用一把鎖。
use crate::infra::cache::REALTIME_TEST_LOCK as TEST_LOCK;

/// 假代號的追蹤條件。
fn trace_targets(symbols: &[&str]) {
    stock_price::set_trace_targets_for_test(
        symbols
            .iter()
            .map(|symbol| PriceTrace::new((*symbol).to_string(), dec!(10), dec!(20)))
            .collect(),
    );
}

/// 換上測試用的 sender，回傳對應的 receiver；結束時呼叫 [`reset`]。
fn install_sender(capacity: usize) -> mpsc::Receiver<PriceUpdateEvent> {
    let (tx, rx) = mpsc::channel(capacity);
    *PRICE_UPDATE_TX.write().unwrap() = Some(tx);
    clear_pending_price_symbols();
    rx
}

fn reset() {
    stop_price_update_consumer_task();
    stock_price::clear_trace_targets_cache();
}

/// 只有被追蹤、價格非零的股票才排入事件；同一檔還在 pending 時不重複排入，
/// 處理完（清掉 pending）才能再排一次。
#[tokio::test]
async fn publish_price_update_enqueues_traced_symbols_once() {
    let _lock = TEST_LOCK.lock().await;
    trace_targets(&["79979T1"]);
    let mut rx = install_sender(8);

    publish_price_update("79979T1".to_string(), Decimal::ZERO);
    publish_price_update("79979T9".to_string(), dec!(15));
    publish_price_updates(vec![
        ("79979T1".to_string(), dec!(15)),
        ("79979T1".to_string(), dec!(16)),
    ]);

    assert_eq!(
        rx.try_recv().map(|event| event.symbol).ok(),
        Some("79979T1".to_string())
    );
    assert!(rx.try_recv().is_err(), "pending 中的股票不應重複排入");
    assert_eq!(pending_price_symbols_len(), 1);

    clear_pending_price_symbol("79979T1");
    publish_price_update("79979T1".to_string(), dec!(17));
    assert!(rx.try_recv().is_ok(), "處理完後可再次排入");

    reset();
}

/// consumer 未啟動或 channel 已滿時丟棄事件，並清掉 pending，下次仍可再排。
#[tokio::test]
async fn publish_price_update_drops_when_it_cannot_send() {
    let _lock = TEST_LOCK.lock().await;
    trace_targets(&["79979T1", "79979T2"]);

    stop_price_update_consumer_task();
    publish_price_update("79979T1".to_string(), dec!(15));
    assert_eq!(
        pending_price_symbols_len(),
        0,
        "沒有 consumer 時不留 pending"
    );

    let mut rx = install_sender(1);
    publish_price_update("79979T1".to_string(), dec!(15));
    publish_price_update("79979T2".to_string(), dec!(15));
    assert_eq!(
        pending_price_symbols_len(),
        1,
        "channel 滿了的那檔不留 pending"
    );
    assert_eq!(
        rx.try_recv().map(|event| event.symbol).ok(),
        Some("79979T1".to_string())
    );

    reset();
}

/// consumer 啟動後會處理事件並清掉 pending；重複啟動不會多開；停止後 task 離開。
#[tokio::test]
async fn price_update_consumer_processes_events_and_stops() {
    let _lock = TEST_LOCK.lock().await;
    reset();
    trace_targets(&["79979T1"]);

    start_price_update_consumer_task();
    start_price_update_consumer_task();
    publish_price_update("79979T1".to_string(), dec!(15));
    // 快取裡沒有這檔的報價，evaluator 只記 trace log，不會觸發警報。
    time::timeout(Duration::from_secs(5), async {
        while pending_price_symbols_len() > 0 {
            task::yield_now().await;
        }
    })
    .await
    .expect("consumer 應在時限內處理完事件");
    assert_eq!(PRICE_CONSUMER_ACTIVE_TASKS.load(Ordering::SeqCst), 1);

    reset();
    time::timeout(Duration::from_secs(5), async {
        while PRICE_CONSUMER_ACTIVE_TASKS.load(Ordering::SeqCst) > 0 {
            task::yield_now().await;
        }
    })
    .await
    .expect("停止後 consumer 應離開");
}

/// 刷新與對帳兩個長週期 task 在等待時收到停止通知就立即離開，不必等週期到期。
#[tokio::test]
async fn refresh_and_reconciliation_tasks_stop_on_notify() {
    let _lock = TEST_LOCK.lock().await;
    start_trace_target_refresh_task();
    start_trace_reconciliation_task();
    // 讓兩個 task 進到等待（current-thread runtime 下 yield 會輪到它們）。
    for _ in 0..10 {
        task::yield_now().await;
    }
    assert!(IS_TARGET_CACHE_REFRESHING.load(Ordering::SeqCst));
    assert!(IS_RECONCILING.load(Ordering::SeqCst));

    stop_trace_target_refresh_task();
    stop_trace_reconciliation_task();
    TRACE_TASK_STOP_NOTIFY.notify_waiters();
    time::timeout(Duration::from_secs(5), wait_for_trace_tasks_to_stop())
        .await
        .expect("等待 task 離開不應逾時");
    assert_eq!(TARGET_REFRESH_ACTIVE_TASKS.load(Ordering::SeqCst), 0);
    assert_eq!(RECONCILIATION_ACTIVE_TASKS.load(Ordering::SeqCst), 0);
}

/// 長週期等待收到停止通知就提前返回。
#[tokio::test]
async fn wait_for_interval_or_stop_returns_on_notify() {
    let _lock = TEST_LOCK.lock().await;
    let waiter = task::spawn(wait_for_interval_or_stop(Duration::from_secs(3600)));
    for _ in 0..5 {
        task::yield_now().await;
    }
    TRACE_TASK_STOP_NOTIFY.notify_waiters();
    time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("收到通知後應立即返回")
        .expect("task 不應 panic");
}

/// 追蹤條件已載入且報價快取有資料就立即返回；否則等到暖身時限才返回。
#[tokio::test(start_paused = true)]
async fn wait_for_price_cache_ready_until_targets_and_snapshots_exist() {
    let _lock = TEST_LOCK.lock().await;
    reset();
    let started = time::Instant::now();
    wait_for_price_cache_ready().await;
    assert!(
        started.elapsed() >= SNAPSHOT_WARMUP_TIMEOUT,
        "未就緒時應等到時限"
    );

    trace_targets(&["79979T1"]);
    SHARE.stock_snapshots.write().unwrap().insert(
        "79979T1".to_string(),
        crate::infra::cache::RealtimeSnapshot::new("79979T1".to_string(), dec!(15)),
    );
    let started = time::Instant::now();
    wait_for_price_cache_ready().await;
    assert!(
        started.elapsed() < SNAPSHOT_WARMUP_POLL_INTERVAL,
        "就緒時應立即返回"
    );

    SHARE.stock_snapshots.write().unwrap().remove("79979T1");
    reset();
}
