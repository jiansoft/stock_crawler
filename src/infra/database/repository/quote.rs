use crate::{
    domain::quote::{
        entity::{
            DailyQuote as DomainDailyQuote, LastDailyQuote as DomainLastDailyQuote,
            QuoteHistoryRecord as DomainQuoteHistoryRecord,
        },
        repository::QuoteRepository,
    },
    infra::{
        database,
        database::table::quote::{
            daily_quote::{self, DailyQuote as TableDailyQuote},
            daily_stock_price_stats::DailyStockPriceStats as TableDailyStockPriceStats,
            last_daily_quotes::LastDailyQuotes as TableLastDailyQuotes,
            quote_history_record::QuoteHistoryRecord as TableQuoteHistoryRecord,
        },
        nosql::redis::CLIENT,
    },
};
use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::RepositoryError;

/// PostgreSQL 實作之報價倉儲。
///
/// 基於 PostgreSQL (SQLx) 與 Redis 實現 `QuoteRepository` 介面，
/// 負責日報價、最新收盤價及全市場估值分布統計的讀寫操作，並在查詢最新報價時封裝雙層快取策略。
#[derive(Default)]
pub struct PgQuoteRepository;

impl PgQuoteRepository {
    /// 建立 `PgQuoteRepository` 新實例。
    pub fn new() -> Self {
        // 傳回全新的 PgQuoteRepository 實例
        PgQuoteRepository
    }

    /// 取得指定的 Redis 快取鍵。
    fn get_cache_key(&self, security_code: &str) -> String {
        format!("LastDailyQuote:{}", security_code)
    }

    /// 嘗試從 Redis 讀取最新收盤價，失敗時靜默回傳 `None`。
    async fn try_cache_get(&self, cache_key: &str) -> Option<DomainLastDailyQuote> {
        let cached_val = CLIENT.get_string(cache_key).await.ok()?;
        serde_json::from_str::<DomainLastDailyQuote>(&cached_val).ok()
    }

    /// 將最新收盤價寫入 Redis 快取（TTL = 86400 秒），失敗時僅記錄 log。
    async fn cache_set(&self, security_code: &str, cache_key: &str, quote: &DomainLastDailyQuote) {
        let Ok(serialized) = serde_json::to_string(quote) else {
            return;
        };
        if let Err(why) = CLIENT.set(cache_key, serialized, 86400).await {
            tracing::error!(
                "Failed to update Redis cache for {}: {:?}",
                security_code,
                why
            );
        }
    }

    /// 批次將最新收盤價寫入 Redis 快取（TTL = 86400 秒），失敗時僅記錄 log。
    async fn batch_cache_set(&self, quotes: &[DomainLastDailyQuote]) {
        for q in quotes {
            let cache_key = self.get_cache_key(&q.stock_symbol);
            let Ok(serialized) = serde_json::to_string(q) else {
                continue;
            };
            if let Err(why) = CLIENT.set(&cache_key, serialized, 86400).await {
                tracing::error!(
                    "Failed to update Redis cache in batch for {}: {:?}",
                    q.stock_symbol,
                    why
                );
            }
        }
    }

    /// 從 PostgreSQL 查詢指定個股的最新收盤價。
    async fn pg_fetch_last_quote(
        &self,
        security_code: &str,
    ) -> Result<Option<DomainLastDailyQuote>, RepositoryError> {
        let sql = r#"
            SELECT date, stock_symbol, closing_price
            FROM last_daily_quotes
            WHERE stock_symbol = $1
        "#;
        let row_opt = sqlx::query_as::<_, TableLastDailyQuotes>(sql)
            .bind(security_code)
            .fetch_optional(database::get_connection())
            .await?;
        Ok(row_opt.map(DomainLastDailyQuote::from))
    }

    /// 執行批次 UPSERT，將最新收盤價寫入 `last_daily_quotes` 資料表。
    async fn pg_save_last_quotes(
        &self,
        quotes: &[DomainLastDailyQuote],
    ) -> Result<(), RepositoryError> {
        let mut dates = Vec::with_capacity(quotes.len());
        let mut symbols = Vec::with_capacity(quotes.len());
        let mut closing_prices = Vec::with_capacity(quotes.len());

        for q in quotes {
            dates.push(q.date);
            symbols.push(q.stock_symbol.clone());
            closing_prices.push(q.closing_price);
        }

        let sql = r#"
            INSERT INTO last_daily_quotes (date, stock_symbol, closing_price, updated_time)
            SELECT * FROM UNNEST($1::date[], $2::varchar[], $3::numeric[])
              AS t(date, stock_symbol, closing_price)
            ON CONFLICT (stock_symbol) DO UPDATE SET
                date = EXCLUDED.date,
                closing_price = EXCLUDED.closing_price,
                updated_time = NOW();
        "#;

        sqlx::query(sql)
            .bind(&dates)
            .bind(&symbols)
            .bind(&closing_prices)
            .execute(database::get_connection())
            .await?;

        Ok(())
    }
}

// === 實體映射實作 ===

impl From<DomainDailyQuote> for TableDailyQuote {
    fn from(domain: DomainDailyQuote) -> Self {
        // 將領域層的每日報價實體轉換為資料庫 Table 模型，完整對齊所有屬性
        TableDailyQuote {
            maximum_price_in_year_date_on: domain.maximum_price_in_year_date_on,
            minimum_price_in_year_date_on: domain.minimum_price_in_year_date_on,
            date: domain.date,
            create_time: domain.create_time,
            record_time: domain.record_time,
            price_earning_ratio: domain.price_earning_ratio,
            moving_average_60: domain.moving_average_60,
            closing_price: domain.closing_price,
            change_range: domain.change_range,
            change: domain.change,
            last_best_bid_price: domain.last_best_bid_price,
            last_best_bid_volume: domain.last_best_bid_volume,
            last_best_ask_price: domain.last_best_ask_price,
            last_best_ask_volume: domain.last_best_ask_volume,
            moving_average_5: domain.moving_average_5,
            moving_average_10: domain.moving_average_10,
            moving_average_20: domain.moving_average_20,
            lowest_price: domain.lowest_price,
            moving_average_120: domain.moving_average_120,
            moving_average_240: domain.moving_average_240,
            maximum_price_in_year: domain.maximum_price_in_year,
            minimum_price_in_year: domain.minimum_price_in_year,
            average_price_in_year: domain.average_price_in_year,
            highest_price: domain.highest_price,
            opening_price: domain.opening_price,
            trading_volume: domain.trading_volume,
            trade_value: domain.trade_value,
            transaction: domain.transaction,
            price_to_book_ratio: domain.price_to_book_ratio,
            stock_symbol: domain.stock_symbol,
            serial: domain.serial,
            year: domain.year,
            month: domain.month,
            day: domain.day,
        }
    }
}

impl From<TableDailyQuote> for DomainDailyQuote {
    fn from(table: TableDailyQuote) -> Self {
        // 將資料庫 Table 模型的每日報價轉換為領域層實體
        DomainDailyQuote {
            serial: table.serial,
            stock_symbol: table.stock_symbol,
            date: table.date,
            opening_price: table.opening_price,
            highest_price: table.highest_price,
            lowest_price: table.lowest_price,
            closing_price: table.closing_price,
            change: table.change,
            change_range: table.change_range,
            trading_volume: table.trading_volume,
            trade_value: table.trade_value,
            transaction: table.transaction,
            last_best_bid_price: table.last_best_bid_price,
            last_best_bid_volume: table.last_best_bid_volume,
            last_best_ask_price: table.last_best_ask_price,
            last_best_ask_volume: table.last_best_ask_volume,
            price_earning_ratio: table.price_earning_ratio,
            price_to_book_ratio: table.price_to_book_ratio,
            moving_average_5: table.moving_average_5,
            moving_average_10: table.moving_average_10,
            moving_average_20: table.moving_average_20,
            moving_average_60: table.moving_average_60,
            moving_average_120: table.moving_average_120,
            moving_average_240: table.moving_average_240,
            maximum_price_in_year: table.maximum_price_in_year,
            minimum_price_in_year: table.minimum_price_in_year,
            average_price_in_year: table.average_price_in_year,
            maximum_price_in_year_date_on: table.maximum_price_in_year_date_on,
            minimum_price_in_year_date_on: table.minimum_price_in_year_date_on,
            year: table.year,
            month: table.month,
            day: table.day,
            create_time: table.create_time,
            record_time: table.record_time,
        }
    }
}

impl From<DomainLastDailyQuote> for TableLastDailyQuotes {
    fn from(domain: DomainLastDailyQuote) -> Self {
        // 將領域層最新收盤價實體轉換為資料庫 Table 模型
        TableLastDailyQuotes {
            date: domain.date,
            closing_price: domain.closing_price,
            stock_symbol: domain.stock_symbol,
        }
    }
}

impl From<TableLastDailyQuotes> for DomainLastDailyQuote {
    fn from(table: TableLastDailyQuotes) -> Self {
        // 將資料庫 Table 模型最新收盤價轉換為領域層實體
        DomainLastDailyQuote {
            date: table.date,
            closing_price: table.closing_price,
            stock_symbol: table.stock_symbol,
        }
    }
}

impl From<DomainQuoteHistoryRecord> for TableQuoteHistoryRecord {
    fn from(domain: DomainQuoteHistoryRecord) -> Self {
        // 將領域層歷史極值紀錄實體轉換為資料庫 Table 模型
        TableQuoteHistoryRecord {
            maximum_price_date_on: domain.maximum_price_date_on,
            minimum_price_date_on: domain.minimum_price_date_on,
            maximum_price_to_book_ratio_date_on: domain.maximum_price_to_book_ratio_date_on,
            minimum_price_to_book_ratio_date_on: domain.minimum_price_to_book_ratio_date_on,
            security_code: domain.security_code,
            maximum_price: domain.maximum_price,
            minimum_price: domain.minimum_price,
            maximum_price_to_book_ratio: domain.maximum_price_to_book_ratio,
            minimum_price_to_book_ratio: domain.minimum_price_to_book_ratio,
        }
    }
}

impl From<TableQuoteHistoryRecord> for DomainQuoteHistoryRecord {
    fn from(table: TableQuoteHistoryRecord) -> Self {
        // 將資料庫 Table 模型歷史極值紀錄轉換為領域層實體
        DomainQuoteHistoryRecord {
            maximum_price_date_on: table.maximum_price_date_on,
            minimum_price_date_on: table.minimum_price_date_on,
            maximum_price_to_book_ratio_date_on: table.maximum_price_to_book_ratio_date_on,
            minimum_price_to_book_ratio_date_on: table.minimum_price_to_book_ratio_date_on,
            security_code: table.security_code,
            maximum_price: table.maximum_price,
            minimum_price: table.minimum_price,
            maximum_price_to_book_ratio: table.maximum_price_to_book_ratio,
            minimum_price_to_book_ratio: table.minimum_price_to_book_ratio,
        }
    }
}

#[async_trait]
impl QuoteRepository for PgQuoteRepository {
    // === 每日報價 (DailyQuote) ===

    async fn save_daily_quote(&self, quote: &DomainDailyQuote) -> Result<()> {
        // 轉換為 Table 實體並呼叫 Table 層的 upsert
        let table_entity = TableDailyQuote::from(quote.clone());
        // 執行 UPSERT 寫入資料庫
        table_entity.upsert().await?;
        Ok(())
    }

    async fn batch_save_daily_quotes(&self, quotes: &[DomainDailyQuote]) -> Result<()> {
        // 將所有領域實體轉換為 Table 實體
        let table_entities: Vec<TableDailyQuote> = quotes
            .iter()
            .map(|q| TableDailyQuote::from(q.clone()))
            .collect();
        // 呼叫 Table 層的 copy_in_raw 批次寫入 PostgreSQL
        TableDailyQuote::copy_in_raw(&table_entities).await?;
        Ok(())
    }

    async fn fetch_quotes_by_date(&self, date: NaiveDate) -> Result<Vec<DomainDailyQuote>> {
        // 讀取指定交易日的所有日報價 Table 資料
        let table_quotes = daily_quote::fetch_daily_quotes_by_date(date).await?;
        // 將所有 Table 實體轉換為領域實體清單
        let domain_quotes = table_quotes
            .into_iter()
            .map(DomainDailyQuote::from)
            .collect();
        Ok(domain_quotes)
    }

    async fn delete_quotes_by_date(&self, date: NaiveDate) -> Result<()> {
        // 刪除指定交易日既有的 DailyQuotes 資料
        sqlx::query(r#"delete from "DailyQuotes" where "Date" = $1;"#)
            .bind(date)
            .execute(database::get_connection())
            .await?;
        Ok(())
    }

    async fn fill_moving_average(&self, quote: &mut DomainDailyQuote) -> Result<()> {
        // 轉換為 Table 實體並呼叫 Table 層的 fill_moving_average 進行資料庫內均線與年內極值計算
        let mut table_entity = TableDailyQuote::from(quote.clone());
        table_entity.fill_moving_average().await?;
        *quote = DomainDailyQuote::from(table_entity);
        Ok(())
    }

    async fn batch_update_moving_average(&self, quotes: &[DomainDailyQuote]) -> Result<()> {
        // 將領域實體列表轉為 Table 實體列表
        let table_entities: Vec<TableDailyQuote> = quotes
            .iter()
            .map(|q| TableDailyQuote::from(q.clone()))
            .collect();
        // 呼叫 Table 層的 batch_update_moving_average 進行批次更新
        TableDailyQuote::batch_update_moving_average(&table_entities).await?;
        Ok(())
    }

    // === 最新報價 (LastDailyQuote) ===

    async fn fetch_last_daily_quotes(&self) -> Result<Vec<DomainLastDailyQuote>> {
        // 從資料庫抓取所有個股的最新收盤價 Table 資料
        let table_quotes = TableLastDailyQuotes::fetch().await?;
        // 將 Table 資料轉換為領域層實體
        let domain_quotes = table_quotes
            .into_iter()
            .map(DomainLastDailyQuote::from)
            .collect();
        Ok(domain_quotes)
    }

    async fn rebuild_last_daily_quotes(&self) -> Result<()> {
        // 呼叫 Table 實作的 rebuild 重建 last_daily_quotes 數據表
        TableLastDailyQuotes::rebuild().await?;
        Ok(())
    }

    async fn fetch_last_quote(&self, security_code: &str) -> Result<Option<DomainLastDailyQuote>> {
        let cache_key = self.get_cache_key(security_code);

        // 1. 嘗試從 Redis 快取讀取
        if let Some(cached) = self.try_cache_get(&cache_key).await {
            return Ok(Some(cached));
        }
        tracing::debug!("Redis cache miss or error for key: {cache_key}, fallback to PostgreSQL");

        // 2. 降級從 PostgreSQL 查詢
        let Some(domain_quote) = self.pg_fetch_last_quote(security_code).await? else {
            return Ok(None);
        };

        // 3. 回寫 Redis 快取（best-effort）
        self.cache_set(security_code, &cache_key, &domain_quote)
            .await;

        Ok(Some(domain_quote))
    }

    async fn save_last_quotes_batch(&self, quotes: &[DomainLastDailyQuote]) -> Result<()> {
        if quotes.is_empty() {
            return Ok(());
        }
        self.pg_save_last_quotes(quotes).await?;
        self.batch_cache_set(quotes).await;
        Ok(())
    }

    // === 股價分布統計 (DailyStockPriceStats) ===

    async fn save_stock_price_stats(&self, date: NaiveDate) -> Result<()> {
        // 呼叫 Table 實作的 upsert 方法計算並保存當日之全市場統計
        TableDailyStockPriceStats::upsert(date, &mut None).await?;
        Ok(())
    }

    async fn makeup_for_the_lack_daily_quotes(&self, date: NaiveDate) -> Result<u64> {
        // 呼叫 Table 實作補齊指定交易日缺漏的收盤資料
        let result = daily_quote::makeup_for_the_lack_daily_quotes(date).await?;
        Ok(result.rows_affected())
    }

    async fn fetch_monthly_stock_price_summary(
        &self,
        security_code: &str,
        year: i32,
        month: i32,
    ) -> Result<Option<(Decimal, Decimal, Decimal)>> {
        // 呼叫 Table 實作查詢指定股票於指定年月的最高、最低、平均收盤價
        let sql = r#"
            SELECT
                MIN("LowestPrice") as lowest_price,
                AVG("ClosingPrice") as avg_price,
                MAX("HighestPrice") as highest_price
            FROM "DailyQuotes"
            WHERE "stock_symbol" = $1 AND "year" = $2 AND "month" = $3
            GROUP BY "stock_symbol", "year", "month";
        "#;
        let row_opt: Option<
            crate::infra::database::table::quote::daily_quote::extension::MonthlyStockPriceSummary,
        > = sqlx::query_as(sql)
            .bind(security_code)
            .bind(year)
            .bind(month)
            .fetch_optional(database::get_connection())
            .await?;

        Ok(row_opt.map(|r| (r.lowest_price, r.avg_price, r.highest_price)))
    }

    // === 歷史極值紀錄 (QuoteHistoryRecord) ===

    async fn fetch_quote_history_records(&self) -> Result<Vec<DomainQuoteHistoryRecord>> {
        // 從資料庫抓取所有個股的歷史價格與股價淨值比極值紀錄 Table 資料
        let table_records = TableQuoteHistoryRecord::fetch().await?;
        // 將 Table 資料轉換為領域層實體
        let domain_records = table_records
            .into_iter()
            .map(DomainQuoteHistoryRecord::from)
            .collect();
        Ok(domain_records)
    }

    async fn save_quote_history_record(&self, record: &DomainQuoteHistoryRecord) -> Result<()> {
        // 將領域實體轉換為 Table 實體並呼叫 Table 層的 upsert 寫入資料庫
        let table_record = TableQuoteHistoryRecord::from(record.clone());
        table_record.upsert().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::quote::entity::LastDailyQuote;
    use rust_decimal_macros::dec;

    #[tokio::test]
    async fn test_cache_aside_flow() {
        // 載入環境變數設定
        dotenv::dotenv().ok();

        if database::ping().await.is_err() {
            println!("跳過 test_cache_aside_flow：無資料庫連接");
            return;
        }

        if crate::infra::nosql::redis::CLIENT.ping().await.is_err() {
            println!("跳過 test_cache_aside_flow：無 Redis 連接");
            return;
        }

        // 建立測試專用的倉儲對象
        let repo = PgQuoteRepository::new();
        let test_symbol = "TEST_9999";

        // 先取得快取的 key
        let cache_key = repo.get_cache_key(test_symbol);
        // 清除先前殘留的 Redis 快取以確保測試獨立性
        let _ = crate::infra::nosql::redis::CLIENT.delete(&cache_key).await;

        // 建立測試用的最新收盤價領域對象
        let test_quote = LastDailyQuote {
            date: NaiveDate::from_ymd_opt(2026, 6, 8).unwrap(),
            stock_symbol: test_symbol.to_string(),
            closing_price: dec!(99.9),
        };

        // 1. 執行批次寫入，內部會同時寫入資料庫與 Redis 快取
        repo.save_last_quotes_batch(std::slice::from_ref(&test_quote))
            .await
            .unwrap();

        // 2. 第一次讀取，此時預期可以直接從快取中命中（因為 batch 寫入時有回寫）
        let fetched_first = repo.fetch_last_quote(test_symbol).await.unwrap();
        assert!(fetched_first.is_some());
        let fetched_first = fetched_first.unwrap();
        assert_eq!(fetched_first.stock_symbol, test_symbol);
        assert_eq!(fetched_first.closing_price, dec!(99.9));

        // 3. 再次清除快取以模擬 Cache Miss 的情境
        let _ = crate::infra::nosql::redis::CLIENT.delete(&cache_key).await;

        // 4. 第二次讀取，因快取已被清除，會觸發 Cache Miss 降級並從 PostgreSQL 重新查詢，最後會回寫快取
        let fetched_miss = repo.fetch_last_quote(test_symbol).await.unwrap();
        assert!(fetched_miss.is_some());
        assert_eq!(fetched_miss.unwrap().closing_price, dec!(99.9));

        // 5. 驗證此時 Redis 快取是否已正確被自動回寫
        let redis_val = crate::infra::nosql::redis::CLIENT
            .get_string(&cache_key)
            .await
            .unwrap();
        let redis_quote: LastDailyQuote = serde_json::from_str(&redis_val).unwrap();
        assert_eq!(redis_quote.stock_symbol, test_symbol);
        assert_eq!(redis_quote.closing_price, dec!(99.9));

        // 6. 清理測試資料（同時清除 Redis 快取與資料庫內的測試列）
        let _ = crate::infra::nosql::redis::CLIENT.delete(&cache_key).await;
        let _ = sqlx::query("DELETE FROM last_daily_quotes WHERE stock_symbol = $1")
            .bind(test_symbol)
            .execute(database::get_connection())
            .await;
    }
}
