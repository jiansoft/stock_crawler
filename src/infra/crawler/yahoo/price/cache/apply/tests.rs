//! 把單一類股的最新快照寫回共用即時快取，並對價格異動的股票發佈 trace 價格事件的測試。

use rust_decimal_macros::dec;

use super::super::TEST_STATE_LOCK;
use super::*;

/// 驗證快取全量更新前，只會針對價格實際異動的股票產生事件。
#[test]
fn test_apply_category_snapshots_counts_changed_prices() {
    let _lock = TEST_STATE_LOCK.blocking_lock();
    SHARE.clear_stock_snapshots();
    // 清空昨日收盤快取，防止並發測試（如 SHARE.load()）將真實 DB 價格寫入，
    // 導致 is_valid_price 以真實昨收價驗證測試用的假價格而誤判為異常。
    SHARE.clear_last_trading_day_quotes();

    let category = YahooClassCategory::enabled(
        crate::infra::crawler::yahoo::YahooClassExchange::Listed,
        40,
        "半導體",
    );
    let mut category_symbols = HashMap::new();

    let mut existing = HashMap::new();
    existing.insert(
        "2330".to_string(),
        RealtimeSnapshot::new("2330".to_string(), dec!(998)),
    );
    SHARE.set_stock_snapshots(existing);

    let mut new_data = HashMap::new();
    new_data.insert(
        "2330".to_string(),
        RealtimeSnapshot::new("2330".to_string(), dec!(1000)),
    );
    new_data.insert(
        "2317".to_string(),
        RealtimeSnapshot::new("2317".to_string(), dec!(180)),
    );
    new_data.insert(
        "2454".to_string(),
        RealtimeSnapshot::new("2454".to_string(), Decimal::ZERO),
    );

    let result = apply_category_snapshots(&category, new_data, &mut category_symbols);

    assert_eq!(result.changed_event_count, 2);

    SHARE.clear_stock_snapshots();
}

/// 驗證同一個類股重新更新時，已不存在的股票會從共用快取中移除。
#[test]
fn test_apply_category_snapshots_replaces_removed_symbols_in_same_category() {
    let _lock = TEST_STATE_LOCK.blocking_lock();
    SHARE.clear_stock_snapshots();
    SHARE.clear_last_trading_day_quotes();

    let category = YahooClassCategory::enabled(
        crate::infra::crawler::yahoo::YahooClassExchange::Listed,
        40,
        "半導體",
    );
    let mut category_symbols = HashMap::new();

    let mut first = HashMap::new();
    first.insert(
        "2330".to_string(),
        RealtimeSnapshot::new("2330".to_string(), dec!(998)),
    );
    first.insert(
        "2303".to_string(),
        RealtimeSnapshot::new("2303".to_string(), dec!(45)),
    );
    apply_category_snapshots(&category, first, &mut category_symbols);

    let mut second = HashMap::new();
    second.insert(
        "2330".to_string(),
        RealtimeSnapshot::new("2330".to_string(), dec!(999)),
    );
    apply_category_snapshots(&category, second, &mut category_symbols);

    let cache = SHARE.stock_snapshots.read().unwrap();
    assert!(cache.contains_key("2330"));
    assert!(!cache.contains_key("2303"));
}
