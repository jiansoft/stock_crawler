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

    /// 持股載入：加總未售出股數、帶出收盤價／產業／ETF 旗標，並為目標中未持有的代號補零股列。
    #[tokio::test]
    #[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
    async fn fetch_positions_joins_prices_and_pads_missing_targets() {
        let pool = database::get_connection();
        let repo = PgPortfolioTargetRepository::with_pool(pool.clone());
        let cleanup = [
            "DELETE FROM stock_ownership_details WHERE member_id=79978",
            "DELETE FROM last_daily_quotes WHERE stock_symbol IN ('79978A','79978B','79978C')",
            "DELETE FROM stocks WHERE stock_symbol IN ('79978A','79978B','79978C')",
        ];
        for sql in cleanup {
            sqlx::query(sql).execute(pool).await.unwrap();
        }
        sqlx::query("INSERT INTO stocks (\"SecurityCode\",\"Name\",stock_symbol,stock_exchange_market_id,stock_industry_id) VALUES ('79978A','甲股','79978A',2,24),('79978B','乙股','79978B',2,24),('79978C','丙股','79978C',2,24)")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO last_daily_quotes(stock_symbol,date,closing_price) VALUES ('79978A','2026-10-23',100),('79978C','2026-10-23',50)")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO stock_ownership_details(member_id,security_code,share_quantity,is_sold) VALUES (79978,'79978A',1000,false),(79978,'79978A',500,false),(79978,'79978B',2000,false),(79978,'79978B',9000,true)")
            .execute(pool).await.unwrap();

        let positions = repo
            .fetch_positions(79978, &["79978A".to_owned(), "79978C".to_owned()])
            .await
            .unwrap();
        let by = |symbol: &str| positions.iter().find(|p| p.symbol == symbol);
        let a = by("79978A").expect("持有 A");
        assert_eq!((a.shares, a.current_price), (1500, 100.0));
        assert_eq!(a.name, "甲股");
        assert!(!a.is_etf);
        // 已售出的不計；B 沒有收盤價時價格為 0，由呼叫端的試算拒絕。
        let b = by("79978B").expect("持有 B");
        assert_eq!((b.shares, b.current_price), (2000, 0.0));
        // 目標中未持有的 C 補上零股列並帶價格。
        let c = by("79978C").expect("補上 C");
        assert_eq!((c.shares, c.current_price), (0, 50.0));
        // 已持有的目標不重複補列。
        assert_eq!(positions.iter().filter(|p| p.symbol == "79978A").count(), 1);

        for sql in cleanup {
            sqlx::query(sql).execute(pool).await.unwrap();
        }
    }

    /// ETF 成分：只取前十大、帶出名稱／權重／資料日期，查無主檔的成分產業為空字串；沒有 ETF 時不查詢。
    #[tokio::test]
    #[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
    async fn fetch_etf_components_returns_top_ten_with_dates() {
        let pool = database::get_connection();
        let repo = PgPortfolioTargetRepository::with_pool(pool.clone());
        let cleanup = [
            "DELETE FROM etf_holding WHERE stock_symbol='79978EF'",
            "DELETE FROM stocks WHERE stock_symbol='79978A'",
        ];
        for sql in cleanup {
            sqlx::query(sql).execute(pool).await.unwrap();
        }
        sqlx::query("INSERT INTO stocks (\"SecurityCode\",\"Name\",stock_symbol,stock_exchange_market_id,stock_industry_id) VALUES ('79978A','甲股','79978A',2,24)")
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO etf_holding(stock_symbol,rank,holding_symbol,holding_name,weight,as_of) VALUES ('79978EF',1,'79978A','甲股',40,'2026-04-30'),('79978EF',2,'79978Z','乙成分',12.5,'2026-04-30'),('79978EF',11,'79978Y','十一名',1,'2026-04-30')")
            .execute(pool).await.unwrap();

        assert!(repo.fetch_etf_components(&[]).await.unwrap().is_empty());
        let components = repo
            .fetch_etf_components(&["79978EF".to_owned()])
            .await
            .unwrap();
        let as_of = chrono::NaiveDate::from_ymd_opt(2026, 4, 30).unwrap();
        let summary: Vec<_> = components
            .iter()
            .map(|dated| {
                (
                    dated.component.stock_symbol.as_str(),
                    dated.component.name.as_str(),
                    dated.component.weight_percent,
                    dated.component.etf_symbol.as_str(),
                    dated.as_of,
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                ("79978A", "甲股", 40.0, "79978EF", as_of),
                ("79978Z", "乙成分", 12.5, "79978EF", as_of),
            ]
        );
        assert!(components[1].component.industry.is_empty());

        for sql in cleanup {
            sqlx::query(sql).execute(pool).await.unwrap();
        }
    }

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
