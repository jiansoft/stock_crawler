use super::entity::MarketIndex;
use anyhow::Result;
use async_trait::async_trait;

/// 市場指數領域的倉儲合約 (Repository Trait)。
///
/// 定義對外部持久化儲存 (如 PostgreSQL) 的資料存取介面，
/// 解耦領域邏輯與基礎設施實作細節。
#[async_trait]
pub trait MarketIndexRepository: Send + Sync {
    /// 取得最近若干筆指數資料（依日期降序排序）。
    ///
    /// # 參數
    /// * `limit` - 限制返回的筆數。
    async fn fetch_latest(&self, limit: usize) -> Result<Vec<MarketIndex>>;

    /// 新增或儲存市場指數實體。
    ///
    /// 若有衝突（同日期與同指數分類），應執行對應的 Upsert 更新策略。
    ///
    /// # 參數
    /// * `market_index` - 指向市場指數實體的引用。
    async fn save(&self, market_index: &MarketIndex) -> Result<()>;
}
