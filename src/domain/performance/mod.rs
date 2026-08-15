/// 績效指標領域實體子模組。
pub mod entity;
/// 排行榜查詢條件與結果子模組。
pub mod query;
/// 績效指標倉儲合約子模組。
pub mod repository;
/// 固定投入金額的報酬模擬器（純函式，無 I/O）。
pub mod simulator;
/// CAGR 計算所需之原始資料來源合約子模組。
pub mod source;

pub use entity::{
    BASE_DATE_GRACE_DAYS, CagrCoverage, CagrMetric, CagrPeriod, CorporateAction, DividendEvent,
    PAR_VALUE, PRINCIPAL, SimulationOutcome, StockCagr,
};
pub use query::{CagrRankingItem, CagrRankingPage, CagrRankingQuery, CagrSortKey};
pub use repository::{CagrRepository, CorporateActionRepository};
pub use source::CagrSourceRepository;
