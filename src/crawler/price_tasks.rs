//! # 即時價格背景任務協調模組
//!
//! 此模組負責統一管理 crawler 層的即時報價背景任務啟停點，
//! 讓事件層不需要直接依賴特定站點的實作模組。
//!
//! 目前由 HiStock 全市場即時報價快取任務作為主要資料生產者。
//! 若未來需要加入其他全市場即時來源，可優先在此模組擴充。

/// 啟動 crawler 層的即時報價背景任務。
///
/// 目前會啟動 HiStock 全市場即時報價快取任務，
/// 由它定期抓取全市場資料並覆蓋更新共用快取。
pub fn start_price_tasks() {
    crate::crawler::histock::price::start_caching_task();
}

/// 停止 crawler 層的即時報價背景任務。
///
/// 目前會停止 HiStock 全市場即時報價快取任務，
/// 並由該模組依既有行為清空共用的即時報價快取。
pub async fn stop_price_tasks() {
    crate::crawler::histock::price::stop_caching_task().await;
}
