/// 績效指標領域實體子模組。
pub mod entity;
/// 績效指標倉儲合約子模組。
pub mod repository;
/// 固定投入金額的報酬模擬器（純函式，無 I/O）。
pub mod simulator;

pub use entity::{
    CagrCoverage, CagrMetric, CagrPeriod, DividendEvent, SimulationOutcome, StockCagr,
};
pub use repository::CagrRepository;
