pub mod backfill;
pub mod calculation;
pub mod event;
// 持股與個人類通知依收件者範圍分送（會員系統計畫 7.6）。
pub mod notify;
// Application 層對外部服務的抽象介面（ports），實作由 interfaces 層註冊。
pub mod ports;
pub mod scheduler;

/// 手動資料回補測試入口。
#[cfg(test)]
pub mod manual_backfill;
