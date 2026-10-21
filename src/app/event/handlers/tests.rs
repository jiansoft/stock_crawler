use super::*;
use std::sync::Arc;
use tokio::sync::Mutex;

/// 建立一個假的事件處理器，將接收到的事件記錄到共用的 Vec 中。
fn fake_handler(log: Arc<Mutex<Vec<DomainEvent>>>) -> EventHandlerFn {
    Box::new(move |event| {
        let log = log.clone();
        Box::pin(async move {
            log.lock().await.push(event);
            Ok(())
        })
    })
}

#[tokio::test]
async fn test_dispatch_stock_registered_event() {
    let event_log = Arc::new(Mutex::new(Vec::new()));
    let dispatcher = EventDispatcher::new_with_handler(Some(fake_handler(event_log.clone())), 10);

    let events = vec![DomainEvent::StockRegistered {
        symbol: "2330".to_string(),
        name: "台積電".to_string(),
        market_id: 2,
        industry_id: 24,
        occurred_at: chrono::Local::now(),
    }];

    dispatcher.dispatch_async(events).await;

    // 等待背景處理迴圈消化事件
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let log = event_log.lock().await;
    assert_eq!(log.len(), 1);
    if let DomainEvent::StockRegistered {
        ref symbol,
        ref name,
        market_id,
        industry_id,
        ..
    } = log[0]
    {
        assert_eq!(symbol, "2330");
        assert_eq!(name, "台積電");
        assert_eq!(market_id, 2);
        assert_eq!(industry_id, 24);
    } else {
        panic!("Expected StockRegistered event");
    }
}

#[tokio::test]
async fn test_dispatch_identity_changed_event() {
    let event_log = Arc::new(Mutex::new(Vec::new()));
    let dispatcher = EventDispatcher::new_with_handler(Some(fake_handler(event_log.clone())), 10);

    let events = vec![DomainEvent::StockIdentityChanged {
        symbol: "2330".to_string(),
        old_name: "台積電".to_string(),
        new_name: "台積電新".to_string(),
        old_market_id: 2,
        new_market_id: 3,
        old_industry_id: 24,
        new_industry_id: 25,
        occurred_at: chrono::Local::now(),
    }];

    dispatcher.dispatch_async(events).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let log = event_log.lock().await;
    assert_eq!(log.len(), 1);
    if let DomainEvent::StockIdentityChanged {
        ref symbol,
        ref old_name,
        ref new_name,
        ..
    } = log[0]
    {
        assert_eq!(symbol, "2330");
        assert_eq!(old_name, "台積電");
        assert_eq!(new_name, "台積電新");
    } else {
        panic!("Expected StockIdentityChanged event");
    }
}

#[tokio::test]
async fn test_dispatch_nav_updated_event() {
    let event_log = Arc::new(Mutex::new(Vec::new()));
    let dispatcher = EventDispatcher::new_with_handler(Some(fake_handler(event_log.clone())), 10);

    let events = vec![DomainEvent::NetAssetValueUpdated {
        symbol: "2330".to_string(),
        old_nav: rust_decimal_macros::dec!(90.0),
        new_nav: rust_decimal_macros::dec!(95.12),
        occurred_at: chrono::Local::now(),
    }];

    dispatcher.dispatch_async(events).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let log = event_log.lock().await;
    assert_eq!(log.len(), 1);
    if let DomainEvent::NetAssetValueUpdated {
        ref symbol,
        old_nav,
        new_nav,
        ..
    } = log[0]
    {
        assert_eq!(symbol, "2330");
        assert_eq!(old_nav, rust_decimal_macros::dec!(90.0));
        assert_eq!(new_nav, rust_decimal_macros::dec!(95.12));
    } else {
        panic!("Expected NetAssetValueUpdated event");
    }
}

#[tokio::test]
async fn test_dispatch_stock_index_updated_event() {
    let event_log = Arc::new(Mutex::new(Vec::new()));
    let dispatcher = EventDispatcher::new_with_handler(Some(fake_handler(event_log.clone())), 10);

    let events = vec![DomainEvent::StockIndexUpdated {
        date: chrono::NaiveDate::from_ymd_opt(2026, 6, 7).unwrap(),
        index: rust_decimal_macros::dec!(16500.25),
        change: rust_decimal_macros::dec!(120.50),
        occurred_at: chrono::Local::now(),
    }];

    dispatcher.dispatch_async(events).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let log = event_log.lock().await;
    assert_eq!(log.len(), 1);
    if let DomainEvent::StockIndexUpdated {
        date,
        index,
        change,
        ..
    } = log[0]
    {
        assert_eq!(date, chrono::NaiveDate::from_ymd_opt(2026, 6, 7).unwrap());
        assert_eq!(index, rust_decimal_macros::dec!(16500.25));
        assert_eq!(change, rust_decimal_macros::dec!(120.50));
    } else {
        panic!("Expected StockIndexUpdated event");
    }
}

#[tokio::test]
async fn test_dispatch_multiple_events() {
    let event_log = Arc::new(Mutex::new(Vec::new()));
    let dispatcher = EventDispatcher::new_with_handler(Some(fake_handler(event_log.clone())), 10);

    let events = vec![
        DomainEvent::StockRegistered {
            symbol: "2330".to_string(),
            name: "台積電".to_string(),
            market_id: 2,
            industry_id: 24,
            occurred_at: chrono::Local::now(),
        },
        DomainEvent::StockIdentityChanged {
            symbol: "2330".to_string(),
            old_name: "台積電".to_string(),
            new_name: "台積電新".to_string(),
            old_market_id: 2,
            new_market_id: 3,
            old_industry_id: 24,
            new_industry_id: 25,
            occurred_at: chrono::Local::now(),
        },
    ];

    dispatcher.dispatch_async(events).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let log = event_log.lock().await;
    assert_eq!(log.len(), 2, "應收到 2 個事件");
}

#[tokio::test]
async fn test_dispatch_empty_events_is_noop() {
    let event_log = Arc::new(Mutex::new(Vec::new()));
    let dispatcher = EventDispatcher::new_with_handler(Some(fake_handler(event_log.clone())), 10);

    // 派發空事件列表
    dispatcher.dispatch_async(vec![]).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let log = event_log.lock().await;
    assert!(log.is_empty(), "空事件列表不應產生 any 處理");
}

#[tokio::test]
async fn test_handler_error_does_not_panic() {
    // 建立一個會回傳錯誤的 handler
    let error_handler: EventHandlerFn =
        Box::new(|_event| Box::pin(async { Err(anyhow::anyhow!("模擬處理失敗")) }));

    let dispatcher = EventDispatcher::new_with_handler(Some(error_handler), 10);

    let events = vec![DomainEvent::StockRegistered {
        symbol: "9999".to_string(),
        name: "測試股".to_string(),
        market_id: 1,
        industry_id: 1,
        occurred_at: chrono::Local::now(),
    }];

    // 即使 handler 回傳錯誤，dispatch 本身不應 panic
    dispatcher.dispatch_async(events).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // 若執行到此處，代表錯誤被妥善處理而非 panic
}
