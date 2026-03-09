//! # 即時價格背景任務協調模組
//!
//! 此模組負責統一管理 crawler 層的即時報價背景任務啟停點，
//! 讓事件層不需要直接依賴特定站點的實作模組。
//!
//! 目前同時管理兩條即時來源：
//! - `HiStock`：既有、已驗證過的全市場即時快取來源。
//! - `Yahoo`：新增的類股輪詢即時快取來源。
//!
//! 協調層只負責統一啟停，不介入各來源自己的抓取與快取策略。

/// 啟動 crawler 層的即時報價背景任務。
///
/// 目前會同時啟動：
/// - HiStock 全市場即時快取任務
/// - Yahoo 類股即時快取任務
pub fn start_price_tasks() {
    // 保留原本已驗證穩定的 HiStock 任務，不因新增 Yahoo 而移除。
    crate::crawler::histock::price::start_caching_task();
    // 再啟動 Yahoo 類股輪詢任務，讓兩個來源都能持續提供即時資料。
    crate::crawler::yahoo::price::start_caching_task();
}

/// 停止 crawler 層的即時報價背景任務。
///
/// 目前會同時停止：
/// - HiStock 全市場即時快取任務
/// - Yahoo 類股即時快取任務
pub async fn stop_price_tasks() {
    // 先停掉 HiStock，保留既有收盤關閉路徑。
    crate::crawler::histock::price::stop_caching_task().await;
    // 再停 Yahoo，避免收盤後仍有類股輪詢在背景更新快取。
    crate::crawler::yahoo::price::stop_caching_task().await;
}
