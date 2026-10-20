/// 市場廣度（站上均線比例、52 週新高新低家數）。
pub mod breadth;
/// 市場指數領域實體模組。
pub mod entity;
/// 加權指數趨勢狀態（均線位置、排列、穿越）。
pub mod regime;
/// 市場指數倉儲介面模組。
pub mod repository;
/// 市場估值水位（上市櫃本益比與股價淨值比每日中位數）。
pub mod valuation;

pub use entity::MarketIndex;
pub use repository::MarketIndexRepository;
