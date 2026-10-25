//! PostgreSQL 實作 [`PortfolioTargetRepository`]。

use anyhow::{Context, Result};
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

    /// 列出明確設定過的維度，包含目標清單為空的全現金設定。
    pub async fn configured_dimensions(&self, member_id: &str) -> Result<Vec<TargetDimension>> {
        let values: Vec<String> = sqlx::query_scalar("SELECT dimension FROM portfolio_target_dimension WHERE member_id=$1 ORDER BY dimension")
            .bind(member_id).fetch_all(self.pool()).await?;
        values
            .into_iter()
            .map(|v| TargetDimension::parse(&v).context("未知的配置維度"))
            .collect()
    }

    /// 取注入的隔離連線池，正式執行時使用全域連線池。
    fn pool(&self) -> &PgPool {
        self.pool
            .as_ref()
            .unwrap_or_else(|| database::get_connection())
    }
}

/// SQLx 載入的目標配置資料列。
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

    async fn has_configuration(&self, member_id: &str, dimension: TargetDimension) -> Result<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portfolio_target_dimension WHERE member_id=$1 AND dimension=$2)")
            .bind(member_id).bind(dimension.as_str()).fetch_one(self.pool()).await?)
    }

    async fn save_targets(
        &self,
        member_id: &str,
        dimension: TargetDimension,
        targets: &[(String, f64)],
    ) -> Result<()> {
        crate::domain::portfolio::target::validate_targets(targets)?;
        let mut tx = self.pool().begin().await?;
        // 即使配置目前為空，也必須序列化同一會員／維度的整份覆蓋更新。
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!(
                "portfolio-target:{member_id}:{}",
                dimension.as_str()
            ))
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO portfolio_target_dimension(member_id, dimension) VALUES ($1,$2) ON CONFLICT(member_id,dimension) DO UPDATE SET updated_time=CURRENT_TIMESTAMP")
            .bind(member_id).bind(dimension.as_str()).execute(&mut *tx).await?;
        // 刪除該使用者在該維度的現有設定
        sqlx::query("DELETE FROM portfolio_target WHERE member_id = $1 AND dimension = $2")
            .bind(member_id)
            .bind(dimension.as_str())
            .execute(&mut *tx)
            .await?;

        // 插入新設定
        for (key, pct) in targets {
            {
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

/// 使用隔離資料庫驗證整份設定的交易語意。
#[cfg(test)]
mod tests {
    use super::*;
    /// 並行覆蓋不得合併成 120%，驗證失敗也不得清掉已存設定。
    #[tokio::test]
    #[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
    async fn targets_replace_atomically_and_preserve_cash_configuration() {
        let repo = PgPortfolioTargetRepository::with_pool(database::get_connection().clone());
        let member = "79979-target-regression";
        for sql in [
            "DELETE FROM portfolio_target WHERE member_id=$1",
            "DELETE FROM portfolio_target_dimension WHERE member_id=$1",
        ] {
            sqlx::query(sql)
                .bind(member)
                .execute(repo.pool())
                .await
                .unwrap();
        }
        assert!(
            !repo
                .has_configuration(member, TargetDimension::Stock)
                .await
                .unwrap()
        );
        let a = vec![("A".to_owned(), 60.0)];
        let b = vec![("B".to_owned(), 60.0)];
        let (a, b) = tokio::join!(
            repo.save_targets(member, TargetDimension::Stock, &a),
            repo.save_targets(member, TargetDimension::Stock, &b)
        );
        a.unwrap();
        b.unwrap();
        let stored = repo
            .list_targets(member, Some(TargetDimension::Stock))
            .await
            .unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].target_percent, 60.0);
        assert!(
            repo.save_targets(
                member,
                TargetDimension::Stock,
                &[
                    ("A".into(), 33.335),
                    ("B".into(), 33.335),
                    ("C".into(), 33.33)
                ]
            )
            .await
            .is_err()
        );
        assert_eq!(
            repo.list_targets(member, Some(TargetDimension::Stock))
                .await
                .unwrap()
                .len(),
            1
        );
        repo.save_targets(member, TargetDimension::Stock, &[("A".into(), 0.0)])
            .await
            .unwrap();
        assert_eq!(
            repo.list_targets(member, Some(TargetDimension::Stock))
                .await
                .unwrap()[0]
                .target_percent,
            0.0
        );
        repo.save_targets(member, TargetDimension::Stock, &[])
            .await
            .unwrap();
        assert!(
            repo.has_configuration(member, TargetDimension::Stock)
                .await
                .unwrap()
        );
        assert!(
            repo.list_targets(member, Some(TargetDimension::Stock))
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            repo.configured_dimensions(member).await.unwrap(),
            [TargetDimension::Stock]
        );
        for sql in [
            "DELETE FROM portfolio_target WHERE member_id=$1",
            "DELETE FROM portfolio_target_dimension WHERE member_id=$1",
        ] {
            sqlx::query(sql)
                .bind(member)
                .execute(repo.pool())
                .await
                .unwrap();
        }
    }
}
