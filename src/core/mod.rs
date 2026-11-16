// 系統告警/通知抽象介面（port），實作由 interfaces 層註冊。
pub mod alert;
pub mod config;
pub mod declare;
pub mod logging;
// 通知項目目錄（會員系統計畫 7.3）：每一種推送對應一個 Topic。
pub mod notify_topic;
pub mod shutdown;
pub mod util;
