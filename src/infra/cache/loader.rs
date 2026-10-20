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

    /// 從資料庫重新載入歷史高低紀錄；失敗只記錄，保留舊快取。
    ///
    /// 依日K重建極值後呼叫，讓記憶體快取與資料庫一致。
    pub async fn reload_quote_history_records(&self) {
        use crate::domain::quote::repository::QuoteRepository;
        match crate::infra::database::repository::quote::PgQuoteRepository::new()
            .fetch_quote_history_records()
            .await
        {
            Ok(records) => self.replace_quote_history_records_cache(records),
            Err(why) => tracing::error!("Failed to reload quote_history_records: {why:?}"),
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
mod tests;
