use crate::domain::registry::entity::Stock;
use crate::domain::registry::repository::StockRepository;
use crate::infra::cache::SHARE;
use crate::infra::database;
use crate::infra::database::table::stock::{self, StockDbRow};
use anyhow::{Context, Result};
use async_trait::async_trait;
use rust_decimal::Decimal;

mod financial;
mod weight;

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
        if let Ok(cache) = SHARE.stocks.read()
            && let Some(cached_stock) = cache.get(symbol)
        {
            return Ok(Some(cached_stock.clone()));
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
    /// 以 `UPDATE … FROM UNNEST` 一次寫回已發行股數與外資持股，成功後同步全域快取。
    /// </summary>
    async fn save_qfii_batch(&self, stocks: &[Stock]) -> Result<u64> {
        if stocks.is_empty() {
            return Ok(0);
        }

        let mut symbols = Vec::with_capacity(stocks.len());
        let mut issued_shares = Vec::with_capacity(stocks.len());
        let mut shares_held = Vec::with_capacity(stocks.len());
        let mut percentages = Vec::with_capacity(stocks.len());
        for stock in stocks {
            symbols.push(stock.symbol().0.clone());
            issued_shares.push(stock.issued_share());
            shares_held.push(stock.qfii_shares_held());
            percentages.push(stock.qfii_share_holding_percentage());
        }

        let sql = r#"
            UPDATE stocks AS s
            SET issued_share = u.issued_share,
                qfii_shares_held = u.qfii_shares_held,
                qfii_share_holding_percentage = u.qfii_share_holding_percentage
            FROM UNNEST($1::text[], $2::bigint[], $3::bigint[], $4::numeric[])
                AS u(stock_symbol, issued_share, qfii_shares_held, qfii_share_holding_percentage)
            WHERE s.stock_symbol = u.stock_symbol;
        "#;
        let updated = sqlx::query(sql)
            .bind(symbols)
            .bind(issued_shares)
            .bind(shares_held)
            .bind(percentages)
            .execute(database::get_connection())
            .await
            .context("Failed to save_qfii_batch in PgStockRepository")?
            .rows_affected();

        if let Ok(mut cache) = SHARE.stocks.write() {
            for stock in stocks {
                cache.insert(stock.symbol().0.clone(), stock.clone());
            }
        }

        Ok(updated)
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

    /// 更新個股最新一季與近四季的 EPS、ROE 等財務指標。
    async fn update_eps_and_roe(&self) -> Result<()> {
        financial::update_eps_and_roe().await
    }

    /// 取得所有每股淨值為零的非下市證券主檔。
    async fn fetch_net_asset_value_per_share_is_zero(&self) -> Result<Vec<Stock>> {
        financial::fetch_net_asset_value_per_share_is_zero().await
    }

    /// 取得指定年度與季別中，缺漏財務報表的證券代號清單。
    async fn fetch_stocks_without_financial_statement(
        &self,
        year: i32,
        quarter: &str,
    ) -> Result<Vec<String>> {
        financial::fetch_stocks_without_financial_statement(year, quarter).await
    }

    /// 將所有有效證券的權值占比重置歸零。
    async fn zeroed_out_weights(&self) -> Result<()> {
        weight::zeroed_out_weights().await
    }

    /// 更新指定證券代號的權值占比。
    async fn update_weight(&self, stock_symbol: &str, weight: Decimal) -> Result<()> {
        weight::update_weight(stock_symbol, weight).await
    }
}

#[cfg(test)]
mod tests;
