use crate::domain::market_index::entity::MarketIndex;
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

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[tokio::test]
    async fn test_pg_market_index_repository_contract() {
        // 這是一個單元合約測試佔位符，如果沒有資料庫連接則跳過
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 PgMarketIndexRepository DB 整合測試：無資料庫連接");
            return;
        }

        let repo = PgMarketIndexRepository::new();
        let test_category = "__TEST_INDEX__";
        let test_date = NaiveDate::from_ymd_opt(2099, 12, 31).unwrap();

        // 1. 清理潛在殘留數據
        sqlx::query("DELETE FROM \"index\" WHERE category = $1 AND date = $2")
            .bind(test_category)
            .bind(test_date)
            .execute(database::get_connection())
            .await
            .ok();

        // 2. 建立新實體並保存
        let market_index = MarketIndex::new(
            test_category.to_string(),
            test_date,
            dec!(16000.5),
            dec!(150.25),
            dec!(5000000000),
            dec!(200000),
            dec!(300000000),
        );

        repo.save(&market_index).await.unwrap();

        // 3. 獲取最新資料並驗證
        let latest = repo.fetch_latest(10).await.unwrap();
        let found = latest
            .iter()
            .find(|idx| idx.category == test_category && idx.date == test_date);
        assert!(found.is_some());
        let found_index = found.unwrap();
        assert_eq!(found_index.index, dec!(16000.5));
        assert_eq!(found_index.change, dec!(150.25));
        assert_eq!(found_index.trade_value, dec!(5000000000));
        assert_eq!(found_index.transaction, dec!(200000));
        assert_eq!(found_index.trading_volume, dec!(300000000));

        // 4. 清理
        sqlx::query("DELETE FROM \"index\" WHERE category = $1 AND date = $2")
            .bind(test_category)
            .bind(test_date)
            .execute(database::get_connection())
            .await
            .unwrap();
    }
}
