//! 短生命週期 TTL 快取。
//!
//! 此模組負責保存「短時間內避免重複處理」的資料，
//! 例如日行情去重與通知節流狀態。

use std::time::{Duration, Instant};

use moka::{sync::Cache, Expiry};
use once_cell::sync::Lazy;
use rust_decimal::Decimal;

/// 全域短時效快取實例。
///
/// 適合儲存短時間內需要去重、節流或通知判斷的資料。
pub static TTL: Lazy<Ttl> = Lazy::new(Default::default);

/// 具 TTL（存活時間）能力的快取容器。
///
/// 目前包含兩類資料：
/// - `daily_quote`：避免同一輪流程重複處理同一筆日行情。
/// - `trace_quote_notify`：記錄通知相關狀態，避免短時間重複通知。
pub struct Ttl {
    /// 每日收盤數據
    daily_quote: Cache<String, TimedValue<String>>,
    trace_quote_notify: Cache<String, TimedValue<Decimal>>,
}

#[derive(Clone)]
struct TimedValue<T> {
    value: T,
    duration: Duration,
}

struct TimedExpiry;

impl<K, T> Expiry<K, TimedValue<T>> for TimedExpiry {
    fn expire_after_create(
        &self,
        _key: &K,
        value: &TimedValue<T>,
        _created_at: Instant,
    ) -> Option<Duration> {
        Some(value.duration)
    }

    fn expire_after_update(
        &self,
        _key: &K,
        value: &TimedValue<T>,
        _updated_at: Instant,
        _duration_until_expiry: Option<Duration>,
    ) -> Option<Duration> {
        Some(value.duration)
    }
}

/// 對 `Ttl` 的操作介面抽象。
///
/// 這個 trait 讓呼叫端可以透過一致 API 操作不同 TTL 區塊，
/// 並把鎖失敗時的降級行為（`None`/`false`）統一封裝在實作層。
pub trait TtlCacheInner {
    /// 清空 `daily_quote` 區塊。
    ///
    /// 目前只清除 `daily_quote`，不影響 `trace_quote_notify`。
    fn clear(&self);
    /// 檢查 `daily_quote` 是否包含指定 key。
    ///
    /// # 參數
    /// - `key`: 要檢查的日行情快取鍵值。
    fn daily_quote_contains_key(&self, key: &str) -> bool;
    /// 讀取 `daily_quote` 的值。
    ///
    /// # 參數
    /// - `key`: 日行情快取鍵值。
    ///
    /// # 回傳
    /// - `Some(String)`：找到資料且尚未過期。
    /// - `None`：未命中、已過期或讀鎖失敗。
    fn daily_quote_get(&self, key: &str) -> Option<String>;
    /// 寫入 `daily_quote`，並設定存活時間。
    ///
    /// # 參數
    /// - `key`: 日行情快取鍵值。
    /// - `val`: 欲寫入的值。
    /// - `duration`: 存活時間。
    ///
    /// # 回傳
    /// - `Some(old_value)`：若原本已有未過期資料。
    /// - `None`：原本無值，或寫入鎖失敗。
    fn daily_quote_set(&self, key: String, val: String, duration: Duration) -> Option<String>;
    /// 檢查 `trace_quote_notify` 是否包含指定 key。
    ///
    /// # 參數
    /// - `key`: 通知節流快取鍵值。
    fn trace_quote_contains_key(&self, key: &str) -> bool;
    /// 讀取 `trace_quote_notify` 的值。
    ///
    /// # 參數
    /// - `key`: 通知節流快取鍵值。
    ///
    /// # 回傳
    /// - `Some(Decimal)`：找到資料且尚未過期。
    /// - `None`：未命中、已過期或讀鎖失敗。
    fn trace_quote_get(&self, key: &str) -> Option<Decimal>;
    /// 寫入 `trace_quote_notify`，並設定存活時間。
    ///
    /// # 參數
    /// - `key`: 通知節流快取鍵值。
    /// - `val`: 欲寫入的數值。
    /// - `duration`: 存活時間。
    ///
    /// # 回傳
    /// - `Some(old_value)`：若原本已有未過期資料。
    /// - `None`：原本無值，或寫入鎖失敗。
    fn trace_quote_set(&self, key: String, val: Decimal, duration: Duration) -> Option<Decimal>;
}

impl TtlCacheInner for Ttl {
    fn clear(&self) {
        self.daily_quote.invalidate_all();
        self.daily_quote.run_pending_tasks();
    }

    fn daily_quote_contains_key(&self, key: &str) -> bool {
        self.daily_quote.contains_key(key)
    }

    fn daily_quote_get(&self, key: &str) -> Option<String> {
        self.daily_quote.get(key).map(|timed| timed.value)
    }

    fn daily_quote_set(&self, key: String, val: String, duration: Duration) -> Option<String> {
        let old_value = self.daily_quote.get(&key).map(|timed| timed.value);
        self.daily_quote.insert(
            key,
            TimedValue {
                value: val,
                duration,
            },
        );
        old_value
    }

    fn trace_quote_contains_key(&self, key: &str) -> bool {
        self.trace_quote_notify.contains_key(key)
    }

    fn trace_quote_get(&self, key: &str) -> Option<Decimal> {
        self.trace_quote_notify.get(key).map(|timed| timed.value)
    }

    fn trace_quote_set(&self, key: String, val: Decimal, duration: Duration) -> Option<Decimal> {
        let old_value = self.trace_quote_notify.get(&key).map(|timed| timed.value);
        self.trace_quote_notify.insert(
            key,
            TimedValue {
                value: val,
                duration,
            },
        );
        old_value
    }
}

impl Ttl {
    /// 建立新的 `Ttl` 容器並配置各區塊初始容量。
    ///
    /// 容量規劃：
    /// - `daily_quote`: 2048
    /// - `trace_quote_notify`: 128
    ///
    /// # 回傳
    /// - `Ttl`：新的 TTL 快取容器。
    pub fn new() -> Self {
        Self {
            daily_quote: Cache::builder()
                .max_capacity(2048)
                .expire_after(TimedExpiry)
                .build(),
            trace_quote_notify: Cache::builder()
                .max_capacity(128)
                .expire_after(TimedExpiry)
                .build(),
        }
    }
}

impl Default for Ttl {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
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
}
