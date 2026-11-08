use crate::domain::market_index::breadth::{BreadthDay, BreadthQuery};
use crate::domain::market_index::entity::MarketIndex;
use crate::domain::market_index::regime::IndexCloseQuery;
use crate::domain::market_index::repository::MarketIndexRepository;
use crate::infra::database;
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Local, NaiveDate};
use rust_decimal::Decimal;
use sqlx::FromRow;

/// 基於 PostgreSQL 的市場指數倉儲實現 (PgMarketIndexRepository)。
///
/// 負責將 `MarketIndex` 領域模型持久化至 `index` 資料表，並將查詢數據還原為領域對象。
pub struct PgMarketIndexRepository;

impl PgMarketIndexRepository {
    /// 建立新的 PgMarketIndexRepository 實例。
    pub fn new() -> Self {
        PgMarketIndexRepository
    }
}

impl Default for PgMarketIndexRepository {
    fn default() -> Self {
        Self::new()
    }
}

/// 資料庫對應的內部資料列結構體，用於 `sqlx::query_as` 映射。
#[derive(FromRow)]
struct IndexDbRow {
    category: String,
    date: NaiveDate,
    index: Decimal,
    change: Decimal,
    trade_value: Decimal,
    transaction: Decimal,
    trading_volume: Decimal,
    create_time: DateTime<Local>,
    update_time: DateTime<Local>,
}

impl From<IndexDbRow> for MarketIndex {
    fn from(row: IndexDbRow) -> Self {
        MarketIndex::reconstitute(
            row.category,
            row.date,
            row.index,
            row.change,
            row.trade_value,
            row.transaction,
            row.trading_volume,
            row.create_time,
            row.update_time,
        )
    }
}

#[async_trait]
impl MarketIndexRepository for PgMarketIndexRepository {
    /// 取得最近的若干筆指數記錄。
    async fn fetch_latest(&self, limit: usize) -> Result<Vec<MarketIndex>> {
        // 使用與原本 Index::fetch 相同的查詢語法，僅加入 limit 參數控制數量。
        let sql = r#"
            SELECT
                category,
                "date",
                trading_volume,
                "transaction",
                trade_value,
                change,
                index,
                create_time,
                update_time
            FROM
                "index"
            ORDER BY
                "date" DESC
            LIMIT $1;
        "#;

        let rows = sqlx::query_as::<_, IndexDbRow>(sql)
            .bind(limit as i64)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch latest market indices from PG")?;

        // 將所有資料列轉換為領域模型後返回。
        Ok(rows.into_iter().map(MarketIndex::from).collect())
    }

    /// 儲存（Upsert）單筆市場指數記錄。
    async fn save(&self, market_index: &MarketIndex) -> Result<()> {
        let sql = r#"
            INSERT INTO "index"
            (
                category,
                "date",
                trading_volume,
                "transaction",
                trade_value,
                change,
                index,
                create_time,
                update_time
            )
            VALUES
            (
                $1, $2, $3, $4, $5, $6, $7, $8, $9
            )
            ON CONFLICT
            (
                "date", category
            )
            DO UPDATE
                SET update_time = EXCLUDED.update_time;
        "#;

        sqlx::query(sql)
            .bind(&market_index.category)
            .bind(market_index.date)
            .bind(market_index.trading_volume)
            .bind(market_index.transaction)
            .bind(market_index.trade_value)
            .bind(market_index.change)
            .bind(market_index.index)
            .bind(market_index.create_time)
            .bind(market_index.update_time)
            .execute(database::get_connection())
            .await
            .context("Failed to save market index to PG")?;

        Ok(())
    }
}

#[async_trait]
impl IndexCloseQuery for PgMarketIndexRepository {
    /// 分類 `category` 在 `until`（含）之前最近 `limit` 個交易日的收盤，由新到舊。
    async fn fetch_index_closes(
        &self,
        category: &str,
        until: Option<NaiveDate>,
        limit: usize,
    ) -> Result<Vec<(NaiveDate, f64)>> {
        let sql = r#"
            SELECT "date", "index"::float8
            FROM "index"
            WHERE category = $1 AND ($2::date IS NULL OR "date" <= $2)
            ORDER BY "date" DESC
            LIMIT $3
        "#;
        sqlx::query_as(sql)
            .bind(category)
            .bind(until)
            .bind(i64::try_from(limit).unwrap_or(i64::MAX))
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch index closes from PG")
    }
}

#[async_trait]
impl BreadthQuery for PgMarketIndexRepository {
    /// 上市櫃合併（市場 id 0）在 `until`（含）之前最近 `limit` 個交易日的市場廣度，由新到舊。
    async fn fetch_breadth(
        &self,
        until: Option<NaiveDate>,
        limit: usize,
    ) -> Result<Vec<BreadthDay>> {
        let sql = r#"
            SELECT date,
                   above_20_day_moving_average, below_20_day_moving_average,
                   above_60_day_moving_average, below_60_day_moving_average,
                   above_240_day_moving_average, below_240_day_moving_average,
                   new_52_week_high, new_52_week_low
            FROM daily_stock_price_stats
            WHERE stock_exchange_market_id = 0 AND ($1::date IS NULL OR date <= $1)
            ORDER BY date DESC
            LIMIT $2
        "#;
        let rows: Vec<(NaiveDate, i32, i32, i32, i32, i32, i32, i32, i32)> = sqlx::query_as(sql)
            .bind(until)
            .bind(i64::try_from(limit).unwrap_or(i64::MAX))
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch market breadth from PG")?;
        Ok(rows
            .into_iter()
            .map(
                |(
                    date,
                    above_ma20,
                    below_ma20,
                    above_ma60,
                    below_ma60,
                    above_ma240,
                    below_ma240,
                    new_high,
                    new_low,
                )| {
                    BreadthDay {
                        date,
                        above_ma20,
                        below_ma20,
                        above_ma60,
                        below_ma60,
                        above_ma240,
                        below_ma240,
                        new_high,
                        new_low,
                    }
                },
            )
            .collect())
    }
}

#[cfg(test)]
mod tests;
