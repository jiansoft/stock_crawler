pub mod backfill;
pub mod calculation;
pub mod event;
/// Application 層對外部服務的抽象介面（ports），實作由 interfaces 層註冊。
pub mod ports;
pub mod scheduler;

/// 手動資料回補測試入口。
#[cfg(test)]
pub mod manual_backfill;
