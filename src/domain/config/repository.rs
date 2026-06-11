use super::entity::SystemConfig;
use anyhow::Result;
use async_trait::async_trait;

/// 系統設定領域的倉儲合約 (Repository Trait)。
///
/// 提供對外部持久化儲存 (如 PostgreSQL) 的設定鍵值對存取介面。
#[async_trait]
pub trait ConfigRepository: Send + Sync {
    /// 依據設定鍵名尋找系統設定。
    ///
    /// # 參數
    /// * `key` - 設定鍵名。
    async fn find_by_key(&self, key: &str) -> Result<Option<SystemConfig>>;

    /// 新增或儲存系統設定（若鍵值衝突，則覆蓋更新）。
    ///
    /// # 參數
    /// * `config` - 系統設定實體引用。
    async fn save(&self, config: &SystemConfig) -> Result<()>;
}
