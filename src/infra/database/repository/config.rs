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
mod tests;
