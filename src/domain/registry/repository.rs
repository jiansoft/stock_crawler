use crate::domain::registry::entity::Stock;
use anyhow::Result;
use async_trait::async_trait;

/// <summary>
/// 證券主檔倉儲特徵介面 (Repository Trait)。
/// 定義對 Stock 聚合根進行持久化查詢與寫入的合約。
/// </summary>
#[async_trait]
pub trait StockRepository: Send + Sync {
    /// <summary>
    /// 依據證券代碼查詢 Stock 聚合根。
    /// </summary>
    async fn find_by_symbol(&self, symbol: &str) -> Result<Option<Stock>>;

    /// <summary>
    /// 新增或更新 Stock 聚合根至持久化儲存。
    /// </summary>
    async fn save(&self, stock: &Stock) -> Result<()>;

    /// <summary>
    /// 獲取所有目前非下市 (有效交易中) 的證券主檔。
    /// </summary>
    async fn fetch_all_active(&self) -> Result<Vec<Stock>>;
}
