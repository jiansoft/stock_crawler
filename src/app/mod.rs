pub mod backfill;
pub mod calculation;
pub mod event;
// 持股與個人類通知依收件者範圍分送（會員系統計畫 7.6）。
pub mod notify;
/// 市場與系統通知分送，包含系統路由快取及故障退回。
pub mod notify_common;
/// 通知送達的佔位、結果紀錄與管道失效處理。
pub mod notify_delivery;
// Application 層對外部服務的抽象介面（ports），實作由 interfaces 層註冊。
pub mod ports;
pub mod scheduler;

/// 手動資料回補測試入口。
#[cfg(test)]
pub mod manual_backfill;
