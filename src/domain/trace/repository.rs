use super::entity::PriceTrace;
use anyhow::Result;
use async_trait::async_trait;

/// 價格追蹤領域的倉儲合約 (Repository Trait)。
///
/// 定義對外部持久化儲存 (如 PostgreSQL) 的價格追蹤設定存取介面。
#[async_trait]
pub trait TraceRepository: Send + Sync {
    /// 取得所有進行監控的價格追蹤設定清單。
    async fn fetch_all(&self) -> Result<Vec<PriceTrace>>;
}
