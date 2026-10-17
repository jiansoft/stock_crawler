//! 具名策略（[`StrategyPresetRepository`]）的 PostgreSQL 實作。

use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::domain::strategy::{StrategyPreset, StrategyPresetRepository};
use crate::infra::database;

/// 全部策略，依名稱排序。
const LIST_SQL: &str =
    "SELECT name::text, query, description, alert FROM strategy_preset ORDER BY name";

/// 指定名稱的策略。
const FIND_SQL: &str =
    "SELECT name::text, query, description, alert FROM strategy_preset WHERE name = $1";

/// 新增或覆蓋，覆蓋時更新修改時間、保留建立時間。
const SAVE_SQL: &str = r#"
INSERT INTO strategy_preset (name, query, description, alert)
VALUES ($1, $2, $3, $4)
ON CONFLICT (name) DO UPDATE
SET query = EXCLUDED.query,
    description = EXCLUDED.description,
    alert = EXCLUDED.alert,
    updated_at = CURRENT_TIMESTAMP
"#;

/// 刪除。
const DELETE_SQL: &str = "DELETE FROM strategy_preset WHERE name = $1";

/// 基於 PostgreSQL `strategy_preset` 表的具名策略倉儲。
#[derive(Debug, Default, Clone, Copy)]
pub struct PgStrategyPresetRepository;

impl PgStrategyPresetRepository {
    /// 建立倉儲。
    pub fn new() -> Self {
        Self
    }
}

type PresetRow = (String, String, String, bool);

fn preset((name, query, description, alert): PresetRow) -> StrategyPreset {
    StrategyPreset {
        name,
        query,
        description,
        alert,
    }
}

#[async_trait]
impl StrategyPresetRepository for PgStrategyPresetRepository {
    async fn list(&self) -> Result<Vec<StrategyPreset>> {
        let rows: Vec<PresetRow> = sqlx::query_as(LIST_SQL)
            .fetch_all(database::get_connection())
            .await
            .context("讀取策略清單失敗")?;
        Ok(rows.into_iter().map(preset).collect())
    }

    async fn find(&self, name: &str) -> Result<Option<StrategyPreset>> {
        let row: Option<PresetRow> = sqlx::query_as(FIND_SQL)
            .bind(name)
            .fetch_optional(database::get_connection())
            .await
            .with_context(|| format!("讀取策略 {name} 失敗"))?;
        Ok(row.map(preset))
    }

    async fn save(&self, preset: &StrategyPreset) -> Result<()> {
        sqlx::query(SAVE_SQL)
            .bind(&preset.name)
            .bind(&preset.query)
            .bind(&preset.description)
            .bind(preset.alert)
            .execute(database::get_connection())
            .await
            .with_context(|| format!("儲存策略 {} 失敗", preset.name))?;
        Ok(())
    }

    async fn delete(&self, name: &str) -> Result<bool> {
        let result = sqlx::query(DELETE_SQL)
            .bind(name)
            .execute(database::get_connection())
            .await
            .with_context(|| format!("刪除策略 {name} 失敗"))?;
        Ok(result.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 新增、覆蓋（保留名稱）、讀取、列出與刪除；用固定的測試名稱並自我清理。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn strategy_preset_round_trip() {
        dotenvy::dotenv().ok();
        let repository = PgStrategyPresetRepository::new();
        let name = "79979 測試策略";
        if repository.delete(name).await.is_err() {
            println!("跳過 strategy_preset_round_trip：無資料庫連接");
            return;
        }
        let mut item = StrategyPreset {
            name: name.to_owned(),
            query: "limit=5&from=2019-01-02".to_owned(),
            description: "測試".to_owned(),
            alert: false,
        };
        repository.save(&item).await.expect("新增");
        item.alert = true;
        item.query = "limit=10&from=2019-01-02".to_owned();
        repository.save(&item).await.expect("覆蓋");
        assert_eq!(
            repository.find(name).await.expect("讀取"),
            Some(item.clone())
        );
        assert!(
            repository
                .list()
                .await
                .expect("列出")
                .iter()
                .any(|preset| preset == &item)
        );
        assert!(repository.delete(name).await.expect("刪除"));
        assert!(!repository.delete(name).await.expect("再刪一次"));
        assert_eq!(repository.find(name).await.expect("讀取"), None);
    }
}
