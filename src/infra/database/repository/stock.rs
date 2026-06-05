use crate::domain::registry::entity::Stock;
use crate::domain::registry::repository::StockRepository;
use crate::infra::cache::SHARE;
use crate::infra::database;
use crate::infra::database::table::stock::{self, StockDbRow};
use anyhow::{Context, Result};
use async_trait::async_trait;

/// <summary>
/// 基於 PostgreSQL 的證券主檔倉儲實現 (PgStockRepository)。
/// 負責將 Stock 聚合根持久化至 PostgreSQL 資料庫，並同步更新記憶體快取與搜尋索引。
/// </summary>
pub struct PgStockRepository;

impl PgStockRepository {
    /// <summary>
    /// 建立新的 PgStockRepository 實例。
    /// </summary>
    pub fn new() -> Self {
        PgStockRepository
    }
}

impl Default for PgStockRepository {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl StockRepository for PgStockRepository {
    /// <summary>
    /// 依據證券代碼查詢 Stock 聚合根。
    /// 優先從快取載入，若未命中則查詢 PostgreSQL 並重構為 Stock 實體。
    /// </summary>
    async fn find_by_symbol(&self, symbol: &str) -> Result<Option<Stock>> {
        // 1. 優先從全域快取取得
        if let Ok(cache) = SHARE.stocks.read() {
            if let Some(cached_stock) = cache.get(symbol) {
                return Ok(Some(cached_stock.clone()));
            }
        }

        // 2. 快取未命中，從 DB 查詢
        let sql = r#"
            SELECT stock_symbol, "Name" AS name, "SuspendListing" AS suspend_listing, 
                   net_asset_value_per_share, weight, return_on_equity, "CreateTime" AS create_time,
                   stock_exchange_market_id, stock_industry_id, issued_share,
                   qfii_shares_held, qfii_share_holding_percentage
            FROM stocks WHERE stock_symbol = $1
        "#;

        let row_opt = sqlx::query_as::<_, StockDbRow>(sql)
            .bind(symbol)
            .fetch_optional(database::get_connection())
            .await
            .context("Failed to query stock by symbol")?;

        if let Some(row) = row_opt {
            let domain_stock = Stock::reconstitute(
                row.stock_symbol,
                row.name,
                row.suspend_listing,
                row.net_asset_value_per_share,
                row.weight,
                row.return_on_equity,
                row.create_time,
                row.stock_exchange_market_id,
                row.stock_industry_id,
                row.issued_share,
                row.qfii_shares_held,
                row.qfii_share_holding_percentage,
            );
            Ok(Some(domain_stock))
        } else {
            Ok(None)
        }
    }

    /// <summary>
    /// 保存或更新 Stock 聚合根。
    /// 執行 Postgres 寫入後，會同步重新建立該股票的搜尋索引並同步更新快取。
    /// </summary>
    async fn save(&self, stock: &Stock) -> Result<()> {
        let sql = r#"
            INSERT INTO stocks (
                stock_symbol, "Name", "CreateTime", "SuspendListing", 
                stock_exchange_market_id, stock_industry_id, weight, net_asset_value_per_share, return_on_equity,
                issued_share, qfii_shares_held, qfii_share_holding_percentage)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            ON CONFLICT (stock_symbol) DO UPDATE SET
                "Name" = EXCLUDED."Name",
                "SuspendListing" = EXCLUDED."SuspendListing",
                stock_exchange_market_id = CASE WHEN EXCLUDED.stock_exchange_market_id > 0 THEN EXCLUDED.stock_exchange_market_id ELSE stocks.stock_exchange_market_id END,
                stock_industry_id = CASE WHEN EXCLUDED.stock_industry_id > 0 THEN EXCLUDED.stock_industry_id ELSE stocks.stock_industry_id END,
                weight = EXCLUDED.weight,
                net_asset_value_per_share = EXCLUDED.net_asset_value_per_share,
                return_on_equity = EXCLUDED.return_on_equity,
                issued_share = EXCLUDED.issued_share,
                qfii_shares_held = EXCLUDED.qfii_shares_held,
                qfii_share_holding_percentage = EXCLUDED.qfii_share_holding_percentage;
        "#;

        sqlx::query(sql)
            .bind(stock.symbol().0.clone())
            .bind(stock.name())
            .bind(stock.created_time())
            .bind(stock.suspend_listing())
            .bind(stock.market_id())
            .bind(stock.industry_id())
            .bind(stock.weight())
            .bind(stock.net_asset_value_per_share())
            .bind(stock.return_on_equity())
            .bind(stock.issued_share())
            .bind(stock.qfii_shares_held())
            .bind(stock.qfii_share_holding_percentage())
            .execute(database::get_connection())
            .await
            .context("Failed to save stock to PostgreSQL")?;

        // 3. 保留搜尋索引建立的副作用
        stock::create_search_index(stock.symbol().0.as_str(), stock.name()).await;

        // 4. 寫入成功，主動同步全域快取 (確保最終一致性)
        if let Ok(mut cache) = SHARE.stocks.write() {
            cache.insert(stock.symbol().0.clone(), stock.clone());
        }

        Ok(())
    }

    /// <summary>
    /// 獲取所有目前非下市 (有效交易中) 的證券主檔。
    /// </summary>
    async fn fetch_all_active(&self) -> Result<Vec<Stock>> {
        let sql = r#"
            SELECT stock_symbol, "Name" AS name, "SuspendListing" AS suspend_listing, 
                   net_asset_value_per_share, weight, return_on_equity, "CreateTime" AS create_time,
                   stock_exchange_market_id, stock_industry_id, issued_share,
                   qfii_shares_held, qfii_share_holding_percentage
            FROM stocks WHERE "SuspendListing" = false
            ORDER BY stock_exchange_market_id, stock_industry_id
        "#;

        let rows = sqlx::query_as::<_, StockDbRow>(sql)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch all active stocks")?;

        let list = rows
            .into_iter()
            .map(|row| {
                Stock::reconstitute(
                    row.stock_symbol,
                    row.name,
                    row.suspend_listing,
                    row.net_asset_value_per_share,
                    row.weight,
                    row.return_on_equity,
                    row.create_time,
                    row.stock_exchange_market_id,
                    row.stock_industry_id,
                    row.issued_share,
                    row.qfii_shares_held,
                    row.qfii_share_holding_percentage,
                )
            })
            .collect();

        Ok(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[tokio::test]
    async fn test_pg_stock_repository_contract() {
        // 這是一個單元合約測試佔位符，如果沒有資料庫連接則跳過
        dotenv::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 PgStockRepository DB 整合測試：無資料庫連接");
            return;
        }

        let repo = PgStockRepository::new();
        let test_symbol = "__TEST_REPO_STOCK__";

        // 清理
        sqlx::query("DELETE FROM stocks WHERE stock_symbol = $1")
            .bind(test_symbol)
            .execute(database::get_connection())
            .await
            .ok();

        let new_stock = Stock::register(test_symbol.to_string(), "測試倉儲".to_string(), 2, 24);

        repo.save(&new_stock).await.unwrap();

        let fetched = repo.find_by_symbol(test_symbol).await.unwrap().unwrap();
        assert_eq!(fetched.symbol().0, test_symbol);
        assert_eq!(fetched.name(), "測試倉儲");
        assert_eq!(fetched.market_id(), 2);
        assert_eq!(fetched.industry_id(), 24);

        // 更新測試
        let mut updated = fetched;
        updated.change_identity("測試倉儲更新".to_string(), 4, 25);
        updated.update_net_asset_value(dec!(100.5));
        repo.save(&updated).await.unwrap();

        let fetched_updated = repo.find_by_symbol(test_symbol).await.unwrap().unwrap();
        assert_eq!(fetched_updated.name(), "測試倉儲更新");
        assert_eq!(fetched_updated.market_id(), 4);
        assert_eq!(fetched_updated.industry_id(), 25);
        assert_eq!(fetched_updated.net_asset_value_per_share(), dec!(100.5));

        // 清理
        sqlx::query("DELETE FROM stocks WHERE stock_symbol = $1")
            .bind(test_symbol)
            .execute(database::get_connection())
            .await
            .unwrap();
    }
}
