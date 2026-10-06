/// 資料健康檢查的量測值。
pub mod entity;
/// 資料健康檢查的倉儲合約。
pub mod repository;

pub use entity::{DataHealthSnapshot, DerivedTableLatest};
pub use repository::DataHealthRepository;
