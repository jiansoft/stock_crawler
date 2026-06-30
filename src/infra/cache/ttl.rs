//! 短生命週期 TTL 快取。
//!
//! 此模組負責保存「短時間內避免重複處理」的資料，
//! 例如日行情去重與通知節流狀態。

use std::time::{Duration, Instant};

use moka::ops::compute;
use moka::{Expiry, sync::Cache};
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
    /// 以 NX 語意原子地寫入 `trace_quote_notify`：僅當 key 不存在（或已過期）時才寫入。
    ///
    /// 相較於「先 `trace_quote_contains_key` 後 `trace_quote_set`」兩步操作，
    /// 此方法對同一 key 是原子的，可避免併發判斷下的競態（TOCTOU）。
    ///
    /// # 參數
    /// - `key`: 通知節流快取鍵值。
    /// - `val`: 欲寫入的數值。
    /// - `duration`: 存活時間。
    ///
    /// # 回傳
    /// - `true`：本次為新寫入（呼叫端可視為「尚未通知過」）。
    /// - `false`：key 已存在且未過期（呼叫端可視為「已通知過」）。
    fn trace_quote_set_if_absent(&self, key: String, val: Decimal, duration: Duration) -> bool;
    /// 依邊界方向，僅當新報價比已記錄的極端值「更極端」時才寫入並回報需通知。
    ///
    /// 與 [`trace_quote_set_if_absent`](Self::trace_quote_set_if_absent) 的「同價去重」不同，
    /// 此方法用於「創新低（或新高）才通知」的節流策略：
    /// - `lower_is_more_extreme == true`（低於最低價 floor）：新價更低時才通知。
    /// - `lower_is_more_extreme == false`（超過最高價 ceiling）：新價更高時才通知。
    ///
    /// 透過 moka 的 `and_compute_with` 對同一 key 原子地完成「讀取-比較-寫入」，
    /// 避免併發下的 TOCTOU 競態。每次成功通知都會以新價格重置 TTL，
    /// 因此時間窗（[`Duration`]）內若無新極端值，則不再重複提醒；
    /// 過了時間窗後 key 失效，相同價格才會再次提醒一次。
    ///
    /// # 參數
    /// - `key`: 通知節流快取鍵值。
    /// - `val`: 目前報價。
    /// - `lower_is_more_extreme`: `true` 表示更低才算更極端（floor）；`false` 表示更高才算（ceiling）。
    /// - `duration`: 寫入後的存活時間。
    ///
    /// # 回傳
    /// - `true`：本次達到新的極端值（或 key 不存在／已過期），呼叫端應發送通知。
    /// - `false`：未比已記錄值更極端，應略過。
    fn trace_quote_notify_if_more_extreme(
        &self,
        key: String,
        val: Decimal,
        lower_is_more_extreme: bool,
        duration: Duration,
    ) -> bool;
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

    fn trace_quote_set_if_absent(&self, key: String, val: Decimal, duration: Duration) -> bool {
        // moka 的 entry API 對同一 key 是原子的：or_insert_with 只在 key 不存在
        // （或已過期）時才會執行初始化並寫入，並透過 is_fresh() 告知是否為新寫入。
        let entry = self
            .trace_quote_notify
            .entry(key)
            .or_insert_with(|| TimedValue {
                value: val,
                duration,
            });
        entry.is_fresh()
    }

    fn trace_quote_notify_if_more_extreme(
        &self,
        key: String,
        val: Decimal,
        lower_is_more_extreme: bool,
        duration: Duration,
    ) -> bool {
        // and_compute_with 對同一 key 是原子的：在閉包內完成「讀取既有極端值 →
        // 比較 → 決定是否覆寫」，避免「先讀後寫」的併發競態。
        let result = self
            .trace_quote_notify
            .entry(key)
            .and_compute_with(|maybe_entry| match maybe_entry {
                Some(entry) => {
                    let prev = entry.into_value().value;
                    let more_extreme = if lower_is_more_extreme {
                        val < prev
                    } else {
                        val > prev
                    };
                    if more_extreme {
                        compute::Op::Put(TimedValue {
                            value: val,
                            duration,
                        })
                    } else {
                        compute::Op::Nop
                    }
                }
                None => compute::Op::Put(TimedValue {
                    value: val,
                    duration,
                }),
            });

        // Inserted：key 原本不存在；ReplacedWith：覆寫成更極端的新值。兩者都代表應通知。
        matches!(
            result,
            compute::CompResult::Inserted(_) | compute::CompResult::ReplacedWith(_)
        )
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
            ttl.trace_quote_notify_if_more_extreme(
                "2330:ceiling".to_string(),
                price,
                false,
                duration,
            )
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

        assert!(ttl.trace_quote_notify_if_more_extreme(
            "k".to_string(),
            dec!(86.0),
            true,
            duration
        ));
        // 未過期且非新低 → 不通知。
        assert!(!ttl.trace_quote_notify_if_more_extreme(
            "k".to_string(),
            dec!(86.0),
            true,
            duration
        ));

        tokio::time::sleep(Duration::from_millis(100)).await;

        // 過期後即使相同價格，也視為首次而再次通知一次。
        assert!(ttl.trace_quote_notify_if_more_extreme(
            "k".to_string(),
            dec!(86.0),
            true,
            duration
        ));
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
}
