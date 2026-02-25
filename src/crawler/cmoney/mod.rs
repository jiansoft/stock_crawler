/// CMoney 爬蟲模組。
///
/// 提供 CMoney 來源的股票即時報價抓取能力。
/// 即時報價
pub mod price;

/// CMoney 站台主機名稱。
const HOST: &str = "www.cmoney.tw";

/// CMoney 資料來源型別標記。
///
/// 實際抓取邏輯透過 `StockInfo` trait 實作提供。
pub struct CMoney {}
