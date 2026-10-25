//! PostgreSQL 實作 [`PortfolioTargetRepository`]。

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use sqlx::{FromRow, PgPool};

use crate::domain::portfolio::target::{
    PortfolioTargetRecord, PortfolioTargetRepository, TargetDimension,
};
use crate::infra::database;

/// PostgreSQL 投資組合目標配置倉儲。
#[derive(Debug, Default, Clone)]
pub struct PgPortfolioTargetRepository {
    pool: Option<PgPool>,
}

impl PgPortfolioTargetRepository {
    /// 建立使用全域連線池之實例。
    pub fn new() -> Self {
        Self { pool: None }
    }

    /// 建立使用指定連線池之實例。
    pub fn with_pool(pool: PgPool) -> Self {
        Self { pool: Some(pool) }
    }

    fn pool(&self) -> &PgPool {
        self.pool
            .as_ref()
            .unwrap_or_else(|| database::get_connection())
    }
}

#[derive(FromRow)]
struct TargetRow {
    member_id: String,
    dimension: String,
    key: String,
    target_percent: Decimal,
    updated_time: DateTime<Utc>,
}

impl TryFrom<TargetRow> for PortfolioTargetRecord {
    type Error = anyhow::Error;

    fn try_from(row: TargetRow) -> Result<Self> {
        let dimension = TargetDimension::parse(&row.dimension)
            .with_context(|| format!("未知的目標維度: {}", row.dimension))?;
        Ok(Self {
            member_id: row.member_id,
            dimension,
            key: row.key,
            target_percent: row.target_percent.to_f64().unwrap_or(0.0),
            updated_time: row.updated_time,
        })
    }
}

#[async_trait]
impl PortfolioTargetRepository for PgPortfolioTargetRepository {
    async fn list_targets(
        &self,
        member_id: &str,
        dimension: Option<TargetDimension>,
    ) -> Result<Vec<PortfolioTargetRecord>> {
        let rows: Vec<TargetRow> = match dimension {
            Some(dim) => {
                sqlx::query_as(
                    r#"SELECT member_id, dimension, key, target_percent, updated_time
                       FROM portfolio_target
                       WHERE member_id = $1 AND dimension = $2
                       ORDER BY target_percent DESC, key ASC"#,
                )
                .bind(member_id)
                .bind(dim.as_str())
                .fetch_all(self.pool())
                .await?
            }
            None => {
                sqlx::query_as(
                    r#"SELECT member_id, dimension, key, target_percent, updated_time
                       FROM portfolio_target
                       WHERE member_id = $1
                       ORDER BY dimension ASC, target_percent DESC, key ASC"#,
                )
                .bind(member_id)
                .fetch_all(self.pool())
                .await?
            }
        };

        rows.into_iter()
            .map(PortfolioTargetRecord::try_from)
            .collect()
    }

    async fn save_targets(
        &self,
        member_id: &str,
        dimension: TargetDimension,
        targets: &[(String, f64)],
    ) -> Result<()> {
        let sum: f64 = targets.iter().map(|(_, pct)| pct).sum();
        if sum > 100.0001 {
            bail!("同一維度的目標權重總和 ({sum:.2}%) 不得超過 100%");
        }
        for (key, pct) in targets {
            if *pct < 0.0 || *pct > 100.0 {
                bail!("目標權重 ({key}: {pct}%) 必須介於 0% 至 100%");
            }
        }

        let mut tx = self.pool().begin().await?;

        // 刪除該使用者在該維度的現有設定
        sqlx::query("DELETE FROM portfolio_target WHERE member_id = $1 AND dimension = $2")
            .bind(member_id)
            .bind(dimension.as_str())
            .execute(&mut *tx)
            .await?;

        // 插入新設定
        for (key, pct) in targets {
            if *pct > 0.0 {
                sqlx::query(
                    r#"INSERT INTO portfolio_target (member_id, dimension, key, target_percent, updated_time)
                       VALUES ($1, $2, $3, $4, CURRENT_TIMESTAMP)"#,
                )
                .bind(member_id)
                .bind(dimension.as_str())
                .bind(key)
                .bind(Decimal::from_f64_retain(*pct).unwrap_or_default())
                .execute(&mut *tx)
                .await?;
            }
        }

        tx.commit().await?;
        Ok(())
    }
}
