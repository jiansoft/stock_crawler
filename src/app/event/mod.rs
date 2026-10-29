// 正式庫停機與重啟監控
pub mod db_health;
// 領域事件處理器與派發器
pub mod handlers;
// 正式站主機（pi）的供電、記憶體與 SD 卡健康檢查
pub mod host_health;
/// 台股事件
pub mod taiwan_stock;
/// 追踪 ex.即時股價是否達到高低標
pub mod trace;

pub use handlers::get_global_dispatcher;
