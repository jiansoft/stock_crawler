//! 短生命週期 TTL 快取。
//!
//! 此模組負責保存「短時間內避免重複處理」的資料，
//! 例如日行情去重與通知節流狀態。

use std::{sync::RwLock, time::Duration};

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
    daily_quote: RwLock<ttl_cache::TtlCache<String, String>>,
    trace_quote_notify: RwLock<ttl_cache::TtlCache<String, Decimal>>,
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
        if let Ok(mut ttl) = self.daily_quote.write() {
            ttl.clear()
        }
    }

    fn daily_quote_contains_key(&self, key: &str) -> bool {
        match self.daily_quote.read() {
            Ok(ttl) => ttl.contains_key(key),
            Err(_) => false,
        }
    }

    fn daily_quote_get(&self, key: &str) -> Option<String> {
        match self.daily_quote.read() {
            Ok(ttl) => ttl.get(key).map(|value| value.to_string()),
            Err(_) => None,
        }
    }

    fn daily_quote_set(&self, key: String, val: String, duration: Duration) -> Option<String> {
        match self.daily_quote.write() {
            Ok(mut ttl) => ttl.insert(key, val, duration),
            Err(_) => None,
        }
    }

    fn trace_quote_contains_key(&self, key: &str) -> bool {
        match self.trace_quote_notify.read() {
            Ok(ttl) => ttl.contains_key(key),
            Err(_) => false,
        }
    }

    fn trace_quote_get(&self, key: &str) -> Option<Decimal> {
        match self.trace_quote_notify.read() {
            Ok(ttl) => ttl.get(key).copied(),
            Err(_) => None,
        }
    }

    fn trace_quote_set(&self, key: String, val: Decimal, duration: Duration) -> Option<Decimal> {
        match self.trace_quote_notify.write() {
            Ok(mut ttl) => ttl.insert(key, val, duration),
            Err(_) => None,
        }
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
            daily_quote: RwLock::new(ttl_cache::TtlCache::new(2048)),
            trace_quote_notify: RwLock::new(ttl_cache::TtlCache::new(128)),
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

    #[tokio::test]
    async fn test_init() {
        dotenv::dotenv().ok();

        let duration = Duration::from_millis(500);
        TTL.daily_quote
            .write()
            .unwrap()
            .insert("1".to_string(), "10".to_string(), duration);

        assert_eq!(TTL.daily_quote_get("1"), Some("10".to_string()));
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(TTL.daily_quote_get("1"), None);
    }
}
