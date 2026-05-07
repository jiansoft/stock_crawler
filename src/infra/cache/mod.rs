//! 全域快取模組。
//!
//! 本模組提供兩類快取：
//! 1. [`SHARE`]：長生命週期的業務資料快取，包含股票主檔、產業分類、指數、
//!    最近月營收、最後交易日收盤價與歷史高低統計。
//! 2. [`TTL`]：短生命週期的暫存快取，適合「短時間內避免重複處理」的場景。
//!
//! 設計上以 `RwLock` 保護共享資料，讀多寫少的路徑可並行讀取。
//! 若鎖取得失敗，多數 API 會回傳 `None` 或 `false` 以避免 panic，
//! 並由上層依回傳值決定是否重試或降級處理。

mod lookup;
mod realtime;
mod share;
mod ttl;

pub use realtime::RealtimeSnapshot;
pub use share::SHARE;
pub use ttl::{TtlCacheInner, TTL};
