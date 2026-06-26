use std::collections::HashMap;

use crate::{
    core::util::map::Keyable,
    domain::market_index::MarketIndex,
    domain::market_index::repository::MarketIndexRepository,
    infra::crawler::share as crawler_share,
    infra::database::{
        repository::market_index::PgMarketIndexRepository,
        table::{last_daily_quotes, revenue, stock},
    },
};

use super::share::Share;

impl Share {
    /// 以新抓到的完整指數清單覆蓋舊快取。
    fn replace_indices_cache(&self, indices: Vec<MarketIndex>) {
        let mut new_cache = HashMap::with_capacity(indices.len());
        for index in indices {
            new_cache.insert(index.key(), index);
        }

        match self.indices.write() {
            Ok(mut cache) => *cache = new_cache,
            Err(why) => {
                tracing::error!("Failed to replace indices cache because {:?}", why);
            }
        }
    }

    /// 以新抓到的完整股票主檔清單覆蓋舊快取。
    fn replace_stocks_cache(&self, stocks: Vec<crate::domain::registry::entity::Stock>) {
        let mut new_cache = HashMap::with_capacity(stocks.len());
        for stock in stocks {
            new_cache.insert(stock.symbol().0.clone(), stock);
        }

        match self.stocks.write() {
            Ok(mut cache) => *cache = new_cache,
            Err(why) => {
                tracing::error!("Failed to replace stocks cache because {:?}", why);
            }
        }
    }

    /// 以新抓到的最近月營收清單覆蓋舊快取。
    fn replace_last_revenues_cache(&self, revenues: Vec<revenue::Revenue>) {
        let mut new_cache = HashMap::new();
        for revenue in revenues {
            let date = revenue.date;
            let stock_symbol = revenue.stock_symbol.to_string();
            new_cache
                .entry(date)
                .or_insert_with(HashMap::new)
                .insert(stock_symbol, revenue);
        }

        match self.last_revenues.write() {
            Ok(mut cache) => *cache = new_cache,
            Err(why) => {
                tracing::error!("Failed to replace last_revenues cache because {:?}", why);
            }
        }
    }

    /// 以新抓到的最後交易日報價清單覆蓋舊快取。
    fn replace_last_trading_day_quotes_cache(
        &self,
        quotes: Vec<last_daily_quotes::LastDailyQuotes>,
    ) {
        let mut new_cache = HashMap::with_capacity(quotes.len());
        for quote in quotes {
            new_cache.insert(quote.stock_symbol.to_string(), quote);
        }

        match self.last_trading_day_quotes.write() {
            Ok(mut cache) => *cache = new_cache,
            Err(why) => {
                tracing::error!(
                    "Failed to replace last_trading_day_quotes cache because {:?}",
                    why
                );
            }
        }
    }

    /// 以新抓到的歷史高低紀錄清單覆蓋舊快取。
    fn replace_quote_history_records_cache(
        &self,
        records: Vec<crate::domain::quote::entity::QuoteHistoryRecord>,
    ) {
        let mut new_cache = HashMap::with_capacity(records.len());
        for record in records {
            new_cache.insert(record.security_code.to_string(), record);
        }

        match self.quote_history_records.write() {
            Ok(mut cache) => *cache = new_cache,
            Err(why) => {
                tracing::error!(
                    "Failed to replace quote_history_records cache because {:?}",
                    why
                );
            }
        }
    }

    /// 從資料庫與外部來源載入主快取資料。
    ///
    /// 載入流程如下：
    /// 1. 載入歷年指數資料到 `indices`。
    /// 2. 載入股票主檔到 `stocks`。
    /// 3. 載入最近兩個月營收到 `last_revenues`（依 `date -> stock_symbol` 分層）。
    /// 4. 載入最後交易日報價到 `last_trading_day_quotes`。
    /// 5. 載入歷史高低統計到 `quote_history_records`。
    /// 6. 嘗試更新目前對外 IP 到 `current_ip`。
    ///
    /// 錯誤處理策略：
    /// - 各段落若失敗會記錄 log，其他段落仍會繼續執行。
    /// - 每一類快取都會以「整批覆蓋」方式刷新，避免舊資料殘留。
    /// - 方法本身不回傳 `Result`，屬於「盡力載入」模型。
    pub async fn load(&self) {
        let index_repo = PgMarketIndexRepository::new();
        match index_repo.fetch_latest(30).await {
            Ok(indices) => self.replace_indices_cache(indices),
            Err(why) => {
                tracing::error!("Failed to fetch indices because {:?}", why);
            }
        }

        match stock::StockDbRow::fetch().await {
            Ok(stocks) => {
                let domain_stocks = stocks.into_iter().map(Into::into).collect();
                self.replace_stocks_cache(domain_stocks);
            }
            Err(why) => {
                tracing::error!("Failed to fetch stocks because {:?}", why);
            }
        }

        match revenue::fetch_last_two_month().await {
            Ok(revenues) => self.replace_last_revenues_cache(revenues),
            Err(why) => {
                tracing::error!("Failed to fetch last_revenues because {:?}", why);
            }
        }

        match last_daily_quotes::LastDailyQuotes::fetch().await {
            Ok(quotes) => self.replace_last_trading_day_quotes_cache(quotes),
            Err(why) => {
                tracing::error!("Failed to fetch last_trading_day_quotes because {:?}", why);
            }
        }

        let quote_repo = crate::infra::database::repository::quote::PgQuoteRepository::new();
        use crate::domain::quote::repository::QuoteRepository;
        match quote_repo.fetch_quote_history_records().await {
            Ok(records) => self.replace_quote_history_records_cache(records),
            Err(why) => {
                tracing::error!("Failed to fetch quote_history_records because {:?}", why);
            }
        }

        // 只有在尚未取得 IP 時才查詢公網 IP，避免在測試或多次載入中重複發起大量網路請求
        if self.get_current_ip().is_none()
            && let Ok(ip) = crawler_share::get_public_ip().await
        {
            self.set_current_ip(ip);
        }

        let current_ip = self.get_current_ip().unwrap_or_default();
        tracing::info!("current_ip  {}", current_ip);
        tracing::info!(
            "CacheShare.indices 初始化 {}",
            self.indices
                .read()
                .map(|cache| cache.len())
                .unwrap_or_default()
        );
        tracing::info!("CacheShare.industries 初始化 {:?}", self.industries);
        tracing::info!(
            "CacheShare.stocks 初始化 {}",
            self.stocks
                .read()
                .map(|cache| cache.len())
                .unwrap_or_default()
        );
        tracing::info!(
            "CacheShare.last_trading_day_quotes 初始化 {}",
            self.last_trading_day_quotes
                .read()
                .map(|cache| cache.len())
                .unwrap_or_default()
        );
        tracing::info!(
            "CacheShare.quote_history_records 初始化 {}",
            self.quote_history_records
                .read()
                .map(|cache| cache.len())
                .unwrap_or_default()
        );

        if let Ok(revenues) = self.last_revenues.read() {
            for revenue in revenues.iter() {
                tracing::info!(
                    "CacheShare.last_revenues 初始化 {}:{}",
                    revenue.0,
                    revenue.1.keys().len()
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use rust_decimal::Decimal;

    use crate::domain::market_index::MarketIndex;
    use crate::infra::database::table::revenue;

    use super::super::share::{SHARE, Share};

    fn make_test_revenue(stock_symbol: &str, date: i64) -> revenue::Revenue {
        let mut r = revenue::Revenue::new();
        r.stock_symbol = stock_symbol.to_string();
        r.date = date;
        r
    }

    fn make_test_index(category: &str, date: NaiveDate) -> MarketIndex {
        MarketIndex::new(
            category.to_string(),
            date,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
        )
    }

    #[test]
    fn test_replace_indices_cache_overwrites_old_entries() {
        let share = Share::new();
        let old_date = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        let new_date = NaiveDate::from_ymd_opt(2025, 2, 1).unwrap();

        share.replace_indices_cache(vec![make_test_index("TAIEX", old_date)]);
        assert!(share.get_stock_index("2025-01-01-TAIEX").is_some());

        share.replace_indices_cache(vec![make_test_index("TAIEX", new_date)]);

        assert!(share.get_stock_index("2025-02-01-TAIEX").is_some());
        assert!(share.get_stock_index("2025-01-01-TAIEX").is_none());
    }

    #[test]
    fn test_replace_last_revenues_cache_overwrites_old_months() {
        let share = Share::new();

        share.replace_last_revenues_cache(vec![
            make_test_revenue("2330", 202501),
            make_test_revenue("2317", 202502),
        ]);
        assert!(share.last_revenues_contains_key(202501, "2330"));
        assert!(share.last_revenues_contains_key(202502, "2317"));

        share.replace_last_revenues_cache(vec![make_test_revenue("2454", 202503)]);

        assert!(!share.last_revenues_contains_key(202501, "2330"));
        assert!(!share.last_revenues_contains_key(202502, "2317"));
        assert!(share.last_revenues_contains_key(202503, "2454"));
    }

    #[tokio::test]
    async fn replace_stocks_cache_controls_stock_lookup_and_contains() {
        let share = Share::new();
        let stock = crate::domain::registry::entity::Stock::register(
            "2330".to_string(),
            "台積電".to_string(),
            0,
            0,
        );

        share.replace_stocks_cache(vec![stock]);

        assert!(share.stock_contains_key("2330"));
        assert!(!share.stock_contains_key("2317"));
        assert_eq!(share.get_stock("2330").await.unwrap().name(), "台積電");
        assert!(share.get_stock("2317").await.is_none());
    }

    #[tokio::test]
    async fn test_get_industry_name() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        assert_eq!(SHARE.get_industry_name(1), Some("水泥工業".to_string()));
        assert_eq!(SHARE.get_industry_name(2), Some("食品工業".to_string()));
        assert_eq!(SHARE.get_industry_name(99), Some("未分類".to_string()));
        assert_eq!(SHARE.get_industry_name(100), None);
    }

    #[tokio::test]
    async fn test_load() {
        dotenv::dotenv().ok();

        SHARE.load().await;

        let mut loop_count = 10;
        for e in SHARE.indices.read().unwrap().iter() {
            if loop_count < 0 {
                break;
            }
            tracing::info!("indices e.date {:?} e.index {:?}", e.1.date, e.1.index);
            loop_count -= 1;
        }

        loop_count = 10;
        for (k, v) in SHARE.stocks.read().unwrap().iter() {
            if loop_count < 0 {
                break;
            }
            tracing::info!("stock {} name {}", k, v.name());
            loop_count -= 1;
        }

        loop_count = 10;
        for (k, v) in SHARE.last_trading_day_quotes.read().unwrap().iter() {
            if loop_count < 0 {
                break;
            }
            tracing::info!("security_code {} closing_price {}", k, v.closing_price);
            loop_count -= 1;
        }

        for (k, v) in SHARE.industries.iter() {
            tracing::info!("name {}  category {}", k, v);
        }

        match SHARE.quote_history_records.write() {
            Ok(mut guard) => {
                if let Some(qhr) = guard.get_mut("2330") {
                    qhr.minimum_price = Decimal::from(1);
                    qhr.maximum_price = Decimal::from(2);
                }
            }
            Err(_) => {}
        }

        for (k, v) in SHARE.quote_history_records.read().unwrap().iter() {
            if k == "2330" {
                dbg!(v);
            }
        }
    }
}
