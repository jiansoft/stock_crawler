use super::*;
use rust_decimal_macros::dec;

/// 驗證 TTL 到期後資料會失效。
#[tokio::test]
async fn daily_quote_expires_after_ttl() {
    let ttl = Ttl::new();
    let duration = Duration::from_millis(500);

    assert_eq!(
        ttl.daily_quote_set("1".to_string(), "10".to_string(), duration),
        None
    );
    assert_eq!(ttl.daily_quote_get("1"), Some("10".to_string()));
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(ttl.daily_quote_get("1"), None);
    assert!(!ttl.daily_quote_contains_key("1"));
}

#[test]
fn set_returns_previous_unexpired_value() {
    let ttl = Ttl::new();
    let duration = Duration::from_secs(60);

    assert_eq!(
        ttl.daily_quote_set("1".to_string(), "10".to_string(), duration),
        None
    );
    assert_eq!(
        ttl.daily_quote_set("1".to_string(), "20".to_string(), duration),
        Some("10".to_string())
    );
    assert_eq!(ttl.daily_quote_get("1"), Some("20".to_string()));

    assert_eq!(
        ttl.trace_quote_set("2".to_string(), dec!(1.23), duration),
        None
    );
    assert_eq!(
        ttl.trace_quote_set("2".to_string(), dec!(4.56), duration),
        Some(dec!(1.23))
    );
    assert_eq!(ttl.trace_quote_get("2"), Some(dec!(4.56)));
}

#[test]
fn clear_only_invalidates_daily_quote() {
    let ttl = Ttl::new();
    let duration = Duration::from_secs(60);

    ttl.daily_quote_set("daily".to_string(), "quote".to_string(), duration);
    ttl.trace_quote_set("trace".to_string(), dec!(9.99), duration);

    ttl.clear();

    assert_eq!(ttl.daily_quote_get("daily"), None);
    assert_eq!(ttl.trace_quote_get("trace"), Some(dec!(9.99)));
}

#[tokio::test]
async fn trace_quote_set_if_absent_is_nx() {
    let ttl = Ttl::new();
    let duration = Duration::from_secs(60);

    // 第一次寫入：key 不存在 → 應為新寫入。
    assert!(ttl.trace_quote_set_if_absent("k".to_string(), dec!(1.0), duration));
    // 同一 key 再寫入：已存在且未過期 → 不應覆寫，回傳 false，且值維持原本的 1.0。
    assert!(!ttl.trace_quote_set_if_absent("k".to_string(), dec!(2.0), duration));
    assert_eq!(ttl.trace_quote_get("k"), Some(dec!(1.0)));
}

#[tokio::test]
async fn trace_quote_set_if_absent_allows_rewrite_after_expiry() {
    let ttl = Ttl::new();
    let duration = Duration::from_millis(50);

    assert!(ttl.trace_quote_set_if_absent("k".to_string(), dec!(1.0), duration));
    tokio::time::sleep(Duration::from_millis(100)).await;
    // 過期後再寫入：應視為新寫入並覆寫為新值。
    assert!(ttl.trace_quote_set_if_absent("k".to_string(), dec!(2.0), duration));
    assert_eq!(ttl.trace_quote_get("k"), Some(dec!(2.0)));
}

#[tokio::test]
async fn trace_quote_notify_if_more_extreme_floor_only_on_new_low() {
    let ttl = Ttl::new();
    let duration = Duration::from_secs(60);
    let notify = |price| {
        ttl.trace_quote_notify_if_more_extreme("2330:floor".to_string(), price, true, duration)
    };

    // 首次：key 不存在 → 通知，基準設為 86.0。
    assert!(notify(dec!(86.0)));
    // 較高價：非新低 → 不通知。
    assert!(!notify(dec!(86.1)));
    assert!(!notify(dec!(86.2)));
    // 創新低 → 通知，基準更新為 85.9。
    assert!(notify(dec!(85.9)));
    // 回到 86.0：非新低 → 不通知。
    assert!(!notify(dec!(86.0)));
    // 相同價格：非更極端 → 不通知。
    assert!(!notify(dec!(85.9)));
}

#[tokio::test]
async fn trace_quote_notify_if_more_extreme_ceiling_only_on_new_high() {
    let ttl = Ttl::new();
    let duration = Duration::from_secs(60);
    let notify = |price| {
        ttl.trace_quote_notify_if_more_extreme("2330:ceiling".to_string(), price, false, duration)
    };

    assert!(notify(dec!(100.0)));
    assert!(!notify(dec!(99.9)));
    assert!(notify(dec!(100.1)));
    assert!(!notify(dec!(100.1)));
}

#[tokio::test]
async fn trace_quote_notify_if_more_extreme_renotifies_after_expiry() {
    let ttl = Ttl::new();
    let duration = Duration::from_millis(50);

    assert!(ttl.trace_quote_notify_if_more_extreme("k".to_string(), dec!(86.0), true, duration));
    // 未過期且非新低 → 不通知。
    assert!(!ttl.trace_quote_notify_if_more_extreme("k".to_string(), dec!(86.0), true, duration));

    tokio::time::sleep(Duration::from_millis(100)).await;

    // 過期後即使相同價格，也視為首次而再次通知一次。
    assert!(ttl.trace_quote_notify_if_more_extreme("k".to_string(), dec!(86.0), true, duration));
}

#[tokio::test]
async fn trace_quote_expires_after_ttl() {
    let ttl = Ttl::new();
    let duration = Duration::from_millis(50);

    assert_eq!(
        ttl.trace_quote_set("trace".to_string(), dec!(9.99), duration),
        None
    );
    assert!(ttl.trace_quote_contains_key("trace"));
    assert_eq!(ttl.trace_quote_get("trace"), Some(dec!(9.99)));

    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(ttl.trace_quote_get("trace"), None);
    assert!(!ttl.trace_quote_contains_key("trace"));
}

#[tokio::test]
async fn expired_value_is_not_returned_as_previous_on_update() {
    let ttl = Ttl::new();
    let duration = Duration::from_millis(50);

    ttl.daily_quote_set("daily".to_string(), "old".to_string(), duration);
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(
        ttl.daily_quote_set("daily".to_string(), "new".to_string(), duration),
        None
    );
    assert_eq!(ttl.daily_quote_get("daily"), Some("new".to_string()));
}
