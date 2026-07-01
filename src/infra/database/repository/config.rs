use crate::domain::config::entity::SystemConfig;
use crate::domain::config::repository::ConfigRepository;
use crate::infra::database;
use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::FromRow;

/// 基於 PostgreSQL 的系統設定倉儲實現 (PgConfigRepository)。
///
/// 負責讀寫資料庫中的 `config` 鍵值對表。
pub struct PgConfigRepository;

impl PgConfigRepository {
    /// 建立新的 PgConfigRepository 實例。
    pub fn new() -> Self {
        PgConfigRepository
    }
}

impl Default for PgConfigRepository {
    fn default() -> Self {
        Self::new()
    }
}

/// 資料庫對應的內部設定資料列結構體。
#[derive(FromRow)]
struct ConfigDbRow {
    key: String,
    val: String,
}

impl From<ConfigDbRow> for SystemConfig {
    fn from(row: ConfigDbRow) -> Self {
        SystemConfig::new(row.key, row.val)
    }
}

#[async_trait]
impl ConfigRepository for PgConfigRepository {
    /// 依設定鍵名尋找系統設定。
    async fn find_by_key(&self, key: &str) -> Result<Option<SystemConfig>> {
        let sql = r#"SELECT key, val FROM config WHERE key = $1;"#;

        let row_opt = sqlx::query_as::<_, ConfigDbRow>(sql)
            .bind(key)
            .fetch_optional(database::get_connection())
            .await
            .context("Failed to query config by key from PostgreSQL")?;

        Ok(row_opt.map(SystemConfig::from))
    }

    /// 儲存系統設定（若鍵值衝突，則覆蓋更新）。
    async fn save(&self, config: &SystemConfig) -> Result<()> {
        let sql = r#"
            INSERT INTO config (key, val)
            VALUES ($1, $2)
            ON CONFLICT (key)
            DO UPDATE SET val = EXCLUDED.val;
        "#;

        sqlx::query(sql)
            .bind(&config.key)
            .bind(&config.val)
            .execute(database::get_connection())
            .await
            .context("Failed to save config to PostgreSQL")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pg_config_repository_contract() {
        // 這是一個單元合約測試佔位符，如果沒有資料庫連接則跳過
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 PgConfigRepository DB 整合測試：無資料庫連接");
            return;
        }

        let repo = PgConfigRepository::new();
        let test_key = "__TEST_CONFIG_KEY__";

        // 1. 清理潛在的殘留測試數據
        sqlx::query("DELETE FROM config WHERE key = $1")
            .bind(test_key)
            .execute(database::get_connection())
            .await
            .ok();

        // 2. 寫入並保存測試設定
        let config = SystemConfig::new(test_key.to_string(), "測試值".to_string());
        repo.save(&config).await.unwrap();

        // 3. 讀取並驗證
        let fetched = repo.find_by_key(test_key).await.unwrap().unwrap();
        assert_eq!(fetched.key, test_key);
        assert_eq!(fetched.val, "測試值");

        // 4. 清理測試數據
        sqlx::query("DELETE FROM config WHERE key = $1")
            .bind(test_key)
            .execute(database::get_connection())
            .await
            .unwrap();
    }
}
