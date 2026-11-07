//! 追蹤條件的邊界判斷、警報去重（記憶體 TTL 與 Redis 雙層）與警報訊息的測試；連外的測試標 #[ignore]。

use rust_decimal_macros::dec;

use super::*;

/// 驗證價格區間判斷邏輯可正確處理雙邊界、單邊界與未設定情況。
#[test]
fn test_is_within_boundary() {
    // 設定高低標 (500 ~ 600)
    let mut trace = PriceTrace {
        stock_symbol: "2330".to_string(),
        floor: dec!(500),
        ceiling: dec!(600),
    };

    // 邊界測試
    assert!(is_within_boundary(&trace, dec!(550)));
    assert!(is_within_boundary(&trace, dec!(500)));
    assert!(is_within_boundary(&trace, dec!(600)));
    assert!(!is_within_boundary(&trace, dec!(499.9)));
    assert!(!is_within_boundary(&trace, dec!(600.1)));

    // 僅設定低標 (>= 500)
    trace.ceiling = Decimal::ZERO;
    assert!(is_within_boundary(&trace, dec!(500)));
    assert!(is_within_boundary(&trace, dec!(1000)));
    assert!(!is_within_boundary(&trace, dec!(499.9)));

    // 僅設定高標 (<= 600)
    trace.floor = Decimal::ZERO;
    trace.ceiling = dec!(600);
    assert!(is_within_boundary(&trace, dec!(600)));
    assert!(is_within_boundary(&trace, dec!(0.1)));
    assert!(!is_within_boundary(&trace, dec!(600.1)));

    // 皆未設定
    trace.ceiling = Decimal::ZERO;
    assert!(is_within_boundary(&trace, dec!(123)));
}

/// 價格在區間內時直接回傳不需通知，不碰任何去重快取。
#[tokio::test]
async fn alert_on_price_boundary_ignores_prices_within_range() {
    let target = PriceTrace {
        stock_symbol: "79984".to_string(),
        floor: dec!(10),
        ceiling: dec!(20),
    };
    let sent = alert_on_price_boundary(target, dec!(15), EvaluationSource::PriceEvent, None)
        .await
        .expect("判斷邊界");
    assert!(!sent);
}

/// 跌破低標第一次通知；同價或較高的價格在時間窗內不再通知，創新低才再通知。
/// 測試用假代號，結束時刪除 Redis 去重紀錄。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn alert_on_price_boundary_notifies_only_on_new_lows() {
    dotenvy::dotenv().ok();
    let redis = &crate::infra::nosql::redis::CLIENT;
    if redis.ping().await.is_err() {
        println!("跳過 alert_on_price_boundary_notifies_only_on_new_lows：無 Redis 連線");
        return;
    }
    let target = PriceTrace {
        stock_symbol: "79984".to_string(),
        floor: dec!(10),
        ceiling: Decimal::ZERO,
    };
    let key = build_trace_notification_key(&target, "floor");
    let _ = redis.delete(&key).await;

    let alert = |price| {
        alert_on_price_boundary(
            target.clone(),
            price,
            EvaluationSource::Reconciliation,
            Some("Test"),
        )
    };
    assert!(alert(dec!(9)).await.expect("第一次跌破"));
    assert!(!alert(dec!(9)).await.expect("同價"));
    assert!(!alert(dec!(9.5)).await.expect("未創新低"));
    assert!(alert(dec!(8.5)).await.expect("創新低"));

    redis.delete(&key).await.expect("清除去重紀錄");
}

#[test]
fn test_build_trace_notification_key_includes_boundary() {
    let trace = PriceTrace {
        stock_symbol: "2330".to_string(),
        floor: dec!(25),
        ceiling: dec!(30),
    };

    // Key 僅含股票與邊界方向，不含價格（價格改以快取的值保存為極端值基準）。
    assert_eq!(
        build_trace_notification_key(&trace, "floor"),
        "Trace:2330-25-30:floor"
    );
    assert_eq!(
        build_trace_notification_key(&trace, "ceiling"),
        "Trace:2330-25-30:ceiling"
    );
}

#[test]
fn test_build_trace_notification_key_distinguishes_boundary() {
    let trace = PriceTrace {
        stock_symbol: "2330".to_string(),
        floor: dec!(25),
        ceiling: dec!(30),
    };

    // 不同邊界方向應產生不同 key；同一方向不論價格皆為同一 key。
    assert_ne!(
        build_trace_notification_key(&trace, "floor"),
        build_trace_notification_key(&trace, "ceiling")
    );
}

#[tokio::test]
async fn test_format_alert_message_includes_source_site() {
    let trace = PriceTrace {
        stock_symbol: "0050".to_string(),
        floor: dec!(0),
        ceiling: dec!(200),
    };

    let msg = format_alert_message(&trace, dec!(650), Some("Fugle")).await;

    assert!(msg.contains("超過最高價"));
    assert!(msg.contains("目前報價:650"));
    assert!(msg.contains("採集站點:Fugle"));
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_format_alert_message() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let trace = PriceTrace {
        stock_symbol: "2330".to_string(),
        floor: dec!(500),
        ceiling: dec!(600),
    };

    // 觸發高標
    let msg = format_alert_message(&trace, dec!(650), Some("HiStock")).await;
    assert!(msg.contains("超過最高價"));
    assert!(msg.contains("目前報價:650"));
    assert!(msg.contains("採集站點:HiStock"));

    // 觸發低標
    let msg = msg_low(&trace, dec!(450), Some("Yahoo")).await;
    assert!(msg.contains("低於最低價"));
    assert!(msg.contains("目前報價:450"));
    assert!(msg.contains("採集站點:Yahoo"));
}

async fn msg_low(target: &PriceTrace, price: Decimal, source_site: Option<&str>) -> String {
    format_alert_message(target, price, source_site).await
}

#[tokio::test]
#[ignore]
async fn test_handle_price() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let trace = PriceTrace {
        stock_symbol: "1303".to_string(),
        floor: dec!(70),
        ceiling: dec!(60),
    };

    let result = alert_on_price_boundary(
        trace,
        dec!(560),
        EvaluationSource::PriceEvent,
        Some("TestSite"),
    )
    .await;
    assert!(result.is_ok());
}
