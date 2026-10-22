//! # ETF 專區
//!
//! 淨值與折溢價、基本資料與分類、持股，以及以還原日線計算的含息報酬。
//! stock_go 的 `/stock/etfs` 讀 `etf_snapshot`（由 [`EtfRepository::refresh_snapshot`] 重建）。

/// 由代號、名稱與來源提示推得的分類（類型、投資地區、題材）。
pub mod classify;
/// ETF 實體。
pub mod entity;
/// 前十大持股兩期之間的差異。
pub mod holding_diff;
/// 含息期間報酬與近 1 年風險指標。
pub mod performance;
/// Data API 讀取 ETF 專區的查詢合約。
pub mod query;
/// ETF 倉儲合約。
pub mod repository;
/// ETF 領域之測試替身；僅在測試建置下編譯。
#[cfg(test)]
pub mod test_double;

pub use classify::{Classification, ClassifyHint, Strategy, classify};
pub use entity::{
    EtfAllocationRecord, EtfHoldingPeriod, EtfHoldingPeriods, EtfHoldingRecord, EtfNavHistoryPoint,
    EtfNavRecord, EtfPortfolio, EtfProfileRecord, EtfWeeklyFlow, PremiumPoint,
};
pub use performance::{EtfNavReturns, EtfPerformance, compute_nav_returns, compute_performance};
pub use repository::EtfRepository;
