/// 市場指數領域實體模組。
pub mod entity;
/// 加權指數趨勢狀態（均線位置、排列、穿越）。
pub mod regime;
/// 市場指數倉儲介面模組。
pub mod repository;

pub use entity::MarketIndex;
pub use repository::MarketIndexRepository;
