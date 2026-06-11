use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

/// 殖利率排行領域的倉儲合約 (Repository Trait)。
///
/// 提供對外部持久化儲存 (如 PostgreSQL) 的殖利率排行資料存取與批次重建介面。
#[async_trait]
pub trait YieldRankRepository: Send + Sync {
    /// 依據指定交易日期，重新計算並重建所有股票的殖利率排行資料。
    ///
    /// # 參數
    /// * `date` - 重建的目標交易日期。
    async fn rebuild_by_date(&self, date: NaiveDate) -> Result<()>;
}
