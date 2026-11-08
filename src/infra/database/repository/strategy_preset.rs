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
mod tests;
