//! PostgreSQL 實作 [`PortfolioTargetRepository`]。

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use sqlx::{FromRow, PgPool};

use crate::domain::portfolio::rebalance::{
    CurrentPosition, DatedEtfComponent, EtfLookThroughComponent, PortfolioPositionQuery,
};
use crate::domain::portfolio::target::{
    PortfolioTargetRecord, PortfolioTargetRepository, TargetDimension,
};
use crate::infra::database;

mod revision;

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

/// 持股與主檔 JOIN 後的資料列。
#[derive(FromRow)]
struct PositionRow {
    stock_symbol: String,
    shares: i64,
    name: String,
    industry_name: String,
    closing_price: Option<f64>,
    is_etf: bool,
}

#[async_trait]
impl PortfolioPositionQuery for PgPortfolioTargetRepository {
    async fn fetch_positions(
        &self,
        member_id: i64,
        extra_symbols: &[String],
    ) -> Result<Vec<CurrentPosition>> {
        let mut rows: Vec<PositionRow> = sqlx::query_as(
            r#"
            WITH held AS (
                SELECT security_code AS stock_symbol, SUM(share_quantity)::bigint AS shares
                FROM stock_ownership_details
                WHERE NOT is_sold AND ($1::bigint = 0 OR member_id = $1)
                GROUP BY security_code
                HAVING SUM(share_quantity) > 0
            )
            SELECT held.stock_symbol::text AS stock_symbol, held.shares,
                COALESCE(e.name, s."Name", held.stock_symbol::text) AS name,
                COALESCE(si.name, '') AS industry_name,
                CASE WHEN q.closing_price > 0 THEN q.closing_price::float8 END AS closing_price,
                (e.stock_symbol IS NOT NULL OR s.stock_industry_id = 9001) AS is_etf
            FROM held
            LEFT JOIN stocks s ON s.stock_symbol = held.stock_symbol
            LEFT JOIN stock_industry si ON si.stock_industry_id = s.stock_industry_id
            LEFT JOIN last_daily_quotes q ON q.stock_symbol = held.stock_symbol
            LEFT JOIN etf_snapshot e ON e.stock_symbol = held.stock_symbol
            ORDER BY held.stock_symbol
            "#,
        )
        .bind(member_id)
        .fetch_all(self.pool())
        .await?;
        let missing: Vec<String> = extra_symbols
            .iter()
            .filter(|symbol| !rows.iter().any(|row| &row.stock_symbol == *symbol))
            .cloned()
            .collect();
        if !missing.is_empty() {
            let extra: Vec<PositionRow> = sqlx::query_as(
                r#"
                SELECT s.stock_symbol::text AS stock_symbol, 0::bigint AS shares,
                    COALESCE(e.name, s."Name", s.stock_symbol::text) AS name,
                    COALESCE(si.name, '') AS industry_name,
                    CASE WHEN q.closing_price > 0 THEN q.closing_price::float8 END AS closing_price,
                    (e.stock_symbol IS NOT NULL OR s.stock_industry_id = 9001) AS is_etf
                FROM stocks s
                LEFT JOIN stock_industry si ON si.stock_industry_id = s.stock_industry_id
                LEFT JOIN last_daily_quotes q ON q.stock_symbol = s.stock_symbol
                LEFT JOIN etf_snapshot e ON e.stock_symbol = s.stock_symbol
                WHERE s.stock_symbol = ANY($1)
                "#,
            )
            .bind(&missing)
            .fetch_all(self.pool())
            .await?;
            rows.extend(extra);
        }
        Ok(rows
            .into_iter()
            .map(|row| CurrentPosition {
                symbol: row.stock_symbol,
                name: row.name,
                shares: row.shares,
                current_price: row.closing_price.unwrap_or(0.0),
                industry: row.industry_name,
                is_etf: row.is_etf,
            })
            .collect())
    }

    async fn fetch_etf_components(&self, etf_symbols: &[String]) -> Result<Vec<DatedEtfComponent>> {
        if etf_symbols.is_empty() {
            return Ok(Vec::new());
        }
        // 再平衡頁（data_api handlers::portfolio_rebalance）與週報共用的成分來源：前十大持股。
        let rows: Vec<EtfComponentRow> = sqlx::query_as(
            r#"
            SELECT h.stock_symbol::text AS etf_symbol,
                   h.holding_symbol::text AS holding_symbol,
                   COALESCE(h.holding_name, h.holding_symbol::text) AS holding_name,
                   h.weight::float8 AS weight, h.as_of,
                   COALESCE(si.name, '') AS industry_name
            FROM etf_holding h
            LEFT JOIN stocks s ON s.stock_symbol = h.holding_symbol
            LEFT JOIN stock_industry si ON si.stock_industry_id = s.stock_industry_id
            WHERE h.stock_symbol = ANY($1) AND h.rank <= 10
            ORDER BY h.stock_symbol, h.rank
            "#,
        )
        .bind(etf_symbols)
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| DatedEtfComponent {
                component: EtfLookThroughComponent {
                    stock_symbol: row.holding_symbol,
                    name: row.holding_name,
                    weight_percent: row.weight,
                    etf_symbol: row.etf_symbol,
                    industry: row.industry_name,
                },
                as_of: row.as_of,
            })
            .collect())
    }
}

/// ETF 前十大成分股資料列。
#[derive(FromRow)]
struct EtfComponentRow {
    etf_symbol: String,
    holding_symbol: String,
    holding_name: String,
    weight: f64,
    industry_name: String,
    as_of: chrono::NaiveDate,
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
        // 覆蓋前先留存目前的整組（原本已設定且內容有變才記），改錯可以還原。
        revision::record_before_save(&mut tx, member_id, dimension, targets).await?;
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

#[cfg(test)]
mod tests;
