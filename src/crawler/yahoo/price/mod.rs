//! # Yahoo 股價採集器
//!
//! 此子模組拆分成三個責任：
//! - `quote_page`：單檔 quote 頁的即時價格查詢與 `StockInfo` 實作。
//! - `class_quote`：類股 JSON API 的 URL 組裝與資料解析。
//! - `cache`：開盤期間的類股輪詢任務，逐分類更新共用即時快取。

// 把 Yahoo 價格功能拆成三個子檔案，目的是把「單檔頁面解析」、
// 「類股 API 抓取」與「背景快取任務」分開，避免 `price.rs`
// 變成過大且不容易定位問題的單檔。
mod cache;
mod class_quote;
mod quote_page;

/// 啟動 Yahoo 類股即時快取背景任務。
///
/// 實際任務定義在 [`cache`] 子模組，這裡僅重新匯出給 crawler 協調層使用。
// 對外只暴露啟停任務的入口，讓上層不需要知道內部子模組結構，
// 之後若再拆檔也不會影響呼叫端。
pub use cache::{start_caching_task, stop_caching_task};
