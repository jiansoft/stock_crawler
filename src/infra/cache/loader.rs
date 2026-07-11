use std::collections::HashMap;

use anyhow::{Context, Result, ensure};

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

/// 主快取載入結果報告。
///
/// [`Share::load_required`] 成功時回傳，讓呼叫端（`main`）可以把載入數量
/// 寫進啟動 log，也能看到哪些「非必要」快取降級失敗。
#[derive(Debug)]
pub struct CacheLoadReport {
    /// 股票主檔載入筆數（必要快取，保證大於 0）。
    pub stock_count: usize,
    /// 最後交易日報價載入筆數（必要快取，允許為 0——全新資料庫尚無行情）。
    pub last_trading_day_quote_count: usize,
    /// 載入失敗而降級的「非必要」快取名稱清單；空表示全部成功。
    pub degraded: Vec<&'static str>,
}

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

    /// 從資料庫與外部來源載入主快取資料，並區分「必要」與「非必要」快取。
    ///
    /// ## 必要快取（任一失敗即回傳 `Err`，呼叫端應中止啟動）
    ///
    /// 1. **股票主檔（stocks）**：所有爬蟲與計算的基準清單。查詢失敗或
    ///    「不合理地為空」都視為致命——空的股票主檔代表 schema/查詢壞掉
    ///    或連錯資料庫，此時讓服務繼續啟動只會悄悄漏抓、漏算。
    /// 2. **最後交易日報價（last_trading_day_quotes）**：漲跌幅計算與價格
    ///    追蹤的比較基準。查詢必須成功；但允許為空（全新資料庫尚未抓過行情，
    ///    此時服務仍應啟動以便開始抓資料），為空時記 warning。
    ///
    /// ## 非必要快取（失敗記 log 並列入 degraded 清單，不影響啟動）
    ///
    /// 歷年指數（indices）、最近兩個月營收（last_revenues）、
    /// 歷史高低統計（quote_history_records）、目前對外 IP（current_ip）。
    ///
    /// 每一類快取都以「整批覆蓋」方式刷新，避免舊資料殘留。
    ///
    /// # Errors
    ///
    /// 必要快取查詢失敗或股票主檔為空時回傳錯誤，錯誤鏈包含失敗環節說明。
    pub async fn load_required(&self) -> Result<CacheLoadReport> {
        // === 必要快取 1：股票主檔 ===
        let stocks = stock::StockDbRow::fetch()
            .await
            .context("載入股票主檔（stocks）失敗")?;
        // 空的主檔代表資料庫內容不正常，fail fast 讓部署流程立即發現。
        ensure!(
            !stocks.is_empty(),
            "股票主檔（stocks）為空，資料庫內容可能異常，中止啟動"
        );
        let stock_count = stocks.len();
        let domain_stocks = stocks.into_iter().map(Into::into).collect();
        self.replace_stocks_cache(domain_stocks);

        // === 必要快取 2：最後交易日報價（查詢必須成功；允許為空）===
        let quotes = last_daily_quotes::LastDailyQuotes::fetch()
            .await
            .context("載入最後交易日報價（last_daily_quotes）失敗")?;
        let last_trading_day_quote_count = quotes.len();
        if last_trading_day_quote_count == 0 {
            // 全新資料庫的正常狀態；既有部署看到這行就要警覺資料可能被清空。
            tracing::warn!("last_trading_day_quotes 快取為空（全新資料庫或行情資料遺失）");
        }
        self.replace_last_trading_day_quotes_cache(quotes);

        // === 非必要快取：失敗只降級，不阻擋啟動 ===
        // degraded 蒐集失敗的快取名稱，讓呼叫端能在啟動 log 一眼看到降級狀態。
        let mut degraded: Vec<&'static str> = Vec::new();

        let index_repo = PgMarketIndexRepository::new();
        match index_repo.fetch_latest(30).await {
            Ok(indices) => self.replace_indices_cache(indices),
            Err(why) => {
                tracing::error!("Failed to fetch indices because {:?}", why);
                degraded.push("indices");
            }
        }

        match revenue::fetch_last_two_month().await {
            Ok(revenues) => self.replace_last_revenues_cache(revenues),
            Err(why) => {
                tracing::error!("Failed to fetch last_revenues because {:?}", why);
                degraded.push("last_revenues");
            }
        }

        let quote_repo = crate::infra::database::repository::quote::PgQuoteRepository::new();
        use crate::domain::quote::repository::QuoteRepository;
        match quote_repo.fetch_quote_history_records().await {
            Ok(records) => self.replace_quote_history_records_cache(records),
            Err(why) => {
                tracing::error!("Failed to fetch quote_history_records because {:?}", why);
                degraded.push("quote_history_records");
            }
        }

        // 只有在尚未取得 IP 時才查詢公網 IP，避免在測試或多次載入中重複發起大量網路請求
        if self.get_current_ip().is_none() {
            match crawler_share::get_public_ip().await {
                Ok(ip) => self.set_current_ip(ip),
                Err(why) => {
                    tracing::error!("Failed to fetch public ip because {:?}", why);
                    degraded.push("current_ip");
                }
            }
        }

        let current_ip = self.get_current_ip().unwrap_or_default();
        tracing::info!("current_ip  {}", current_ip);

        self.log_cache_summary();

        Ok(CacheLoadReport {
            stock_count,
            last_trading_day_quote_count,
            degraded,
        })
    }

    /// 從資料庫與外部來源載入主快取資料（best-effort 版本）。
    ///
    /// 這是 [`Self::load_required`] 的寬鬆包裝：載入失敗只記錄 log，不回傳錯誤。
    /// 保留此方法是為了測試與工具情境——它們在資料庫不可用時仍希望繼續執行。
    /// **正式啟動路徑（main）請改用 `load_required`**，必要快取失敗時應中止啟動。
    pub async fn load(&self) {
        if let Err(why) = self.load_required().await {
            tracing::error!("best-effort cache load failed: {:#}", why);
        }
    }

    /// 把各快取目前的載入數量寫進 log，方便啟動時檢視。
    fn log_cache_summary(&self) {
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
    async fn test_replace_last_trading_day_quotes_cache_overwrites() {
        use crate::infra::database::table::last_daily_quotes::LastDailyQuotes;
        use rust_decimal_macros::dec;

        let share = Share::new();

        let mut q1 = LastDailyQuotes::new();
        q1.stock_symbol = "2330".to_string();
        q1.closing_price = dec!(500);
        q1.date = NaiveDate::from_ymd_opt(2025, 1, 2).unwrap();

        let mut q2 = LastDailyQuotes::new();
        q2.stock_symbol = "2317".to_string();
        q2.closing_price = dec!(100);

        share.replace_last_trading_day_quotes_cache(vec![q1, q2]);

        assert!(share.get_stock_last_price("2330").await.is_some());
        assert!(share.get_stock_last_price("2317").await.is_some());
        assert!(share.get_stock_last_price("2454").await.is_none());

        let mut q3 = LastDailyQuotes::new();
        q3.stock_symbol = "2454".to_string();
        q3.closing_price = dec!(200);

        share.replace_last_trading_day_quotes_cache(vec![q3]);

        assert!(share.get_stock_last_price("2330").await.is_none());
        assert!(share.get_stock_last_price("2454").await.is_some());
    }

    #[tokio::test]
    async fn test_set_stock_last_price_updates_existing_entry() {
        use crate::domain::quote::entity::DailyQuote;
        use crate::infra::database::table::last_daily_quotes::LastDailyQuotes;
        use rust_decimal_macros::dec;

        let share = Share::new();

        let mut q = LastDailyQuotes::new();
        q.stock_symbol = "2330".to_string();
        q.closing_price = dec!(500);
        q.date = NaiveDate::from_ymd_opt(2025, 1, 2).unwrap();

        share.replace_last_trading_day_quotes_cache(vec![q]);

        let before = share.get_stock_last_price("2330").await.unwrap();
        assert_eq!(before.closing_price, dec!(500));

        let new_date = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        // 初始化 DailyQuote 並覆寫部分預設欄位
        let dq = DailyQuote {
            stock_symbol: "2330".to_string(),
            closing_price: dec!(620),
            date: new_date,
            ..DailyQuote::default()
        };

        share.set_stock_last_price(&dq).await;

        let after = share.get_last_trading_day_quotes("2330").await.unwrap();
        assert_eq!(after.closing_price, dec!(620));
        assert_eq!(after.date, new_date);
    }

    #[test]
    fn test_replace_quote_history_records_cache_overwrites() {
        use crate::domain::quote::entity::QuoteHistoryRecord;
        use rust_decimal_macros::dec;

        let share = Share::new();

        let mut r1 = QuoteHistoryRecord::new("2330".to_string());
        r1.maximum_price = dec!(600);
        r1.minimum_price = dec!(400);

        share.replace_quote_history_records_cache(vec![r1]);

        {
            let got = share.quote_history_records.read().unwrap();
            let r = got.get("2330").unwrap();
            assert_eq!(r.maximum_price, dec!(600));
            assert_eq!(r.minimum_price, dec!(400));
        }

        let r2 = QuoteHistoryRecord::new("2454".to_string());
        share.replace_quote_history_records_cache(vec![r2]);

        let got = share.quote_history_records.read().unwrap();
        assert!(got.get("2330").is_none());
        assert!(got.get("2454").is_some());
    }

    #[tokio::test]
    async fn test_get_industry_name() {
        dotenvy::dotenv().ok();
        SHARE.load().await;

        assert_eq!(SHARE.get_industry_name(1), Some("水泥工業".to_string()));
        assert_eq!(SHARE.get_industry_name(2), Some("食品工業".to_string()));
        assert_eq!(SHARE.get_industry_name(99), Some("未分類".to_string()));
        assert_eq!(SHARE.get_industry_name(100), None);
    }

    /// 驗證 `load_required` 的必要/非必要快取合約。
    ///
    /// 不同環境的預期：
    /// - 資料庫不可用：應回傳 `Err`（必要快取查詢失敗），而不是默默成功。
    /// - 資料庫可用且 stocks 有資料（本機）：`Ok` 且 `stock_count > 0`。
    /// - 資料庫可用但 stocks 為空（CI 只建 schema 未 seed）：應回傳 `Err`
    ///   且錯誤訊息點名股票主檔——這正是「空的核心快取要 fail fast」的行為。
    #[tokio::test]
    async fn test_load_required_contract() {
        dotenvy::dotenv().ok();

        match SHARE.load_required().await {
            Ok(report) => {
                // 成功即代表必要快取保證成立：股票主檔非空。
                assert!(report.stock_count > 0);
                println!(
                    "load_required ok: stocks={}, quotes={}, degraded={:?}",
                    report.stock_count, report.last_trading_day_quote_count, report.degraded
                );
            }
            Err(why) => {
                // 失敗必須可歸因於必要快取（連線失敗或股票主檔為空），
                // 錯誤訊息應點名失敗環節，方便啟動時定位。
                let msg = format!("{why:#}");
                assert!(
                    msg.contains("股票主檔") || msg.contains("最後交易日報價"),
                    "unexpected error: {msg}"
                );
                println!("load_required err (acceptable in this environment): {msg}");
            }
        }
    }

    #[tokio::test]
    async fn test_load() {
        dotenvy::dotenv().ok();

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

        // 由於 guard 的 mutable 借用生命週期限制，此處無法直接合併 nested if let
        #[allow(clippy::collapsible_if)]
        if let Ok(mut guard) = SHARE.quote_history_records.write() {
            if let Some(qhr) = guard.get_mut("2330") {
                qhr.minimum_price = Decimal::from(1);
                qhr.maximum_price = Decimal::from(2);
            }
        }

        for (k, v) in SHARE.quote_history_records.read().unwrap().iter() {
            if k == "2330" {
                dbg!(v);
            }
        }
    }
}
