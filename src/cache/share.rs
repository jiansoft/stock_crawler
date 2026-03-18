//! 長生命週期主快取。
//!
//! 此模組負責維護 crawler 執行期間會反覆共用的主資料快取，
//! 包含股票主檔、最後交易日行情、歷史高低統計、月營收索引、
//! 即時報價快照，以及產業與交易市場對照資訊。

use std::{collections::HashMap, sync::RwLock};

use once_cell::sync::Lazy;
use rust_decimal::Decimal;

use super::{
    lookup::{default_exchange_markets, default_industries},
    realtime::RealtimeSnapshot,
};
use crate::crawler::share as crawler_share;
use crate::{
    database::table::{
        daily_quote, index, last_daily_quotes, quote_history_record, revenue, stock,
        stock_exchange_market,
    },
    logging,
    util::map::Keyable,
};

/// 全域共享資料快取實例。
///
/// 這是整個 crawler 程式在執行期間共用的主快取容器。
/// 請在服務啟動時先呼叫 [`Share::load`] 完成初始化，再進行讀取。
pub static SHARE: Lazy<Share> = Lazy::new(Default::default);

/// 各類長生命週期快取的集中管理器。
///
/// 主要用途：
/// - 讓不同 crawler / backfill / event 流程共用同一份資料，降低重複查詢成本。
/// - 提供具型別的快取查詢與更新入口，避免散落在各模組中直接操作 `HashMap`。
/// - 在讀鎖失敗時以安全預設值降級，降低流程中斷風險。
///
/// 注意：
/// - `new()` 只建立空容器與靜態對照表，不會觸發資料庫或網路 I/O。
/// - 真正載入資料需呼叫 [`Self::load`]。
pub struct Share {
    /// 存放台股歷年指數
    indices: RwLock<HashMap<String, index::Index>>,
    /// 存放台股股票代碼
    pub stocks: RwLock<HashMap<String, stock::Stock>>,
    /// 月營收的快取(防止重複寫入)，第一層 Key:日期 yyyyMM 第二層 Key:股號
    last_revenues: RwLock<HashMap<i64, HashMap<String, revenue::Revenue>>>,
    /// 存放最後交易日股票報價數據
    last_trading_day_quotes: RwLock<HashMap<String, last_daily_quotes::LastDailyQuotes>>,
    /// 股票歷史、淨值比等最高與最低數據。
    ///
    /// 啟動時會先從資料庫載入；若後續抓到更新資料，應同步更新資料庫與這份快取。
    pub quote_history_records: RwLock<HashMap<String, quote_history_record::QuoteHistoryRecord>>,
    /// 股票產業分類
    industries: HashMap<String, i32>,
    /// 股票產業分類(2, 'TAI', '上市', 1),(4, 'TWO', '上櫃', 2), (5, 'TWE', '興櫃', 2);
    exchange_markets: HashMap<i32, stock_exchange_market::StockExchangeMarket>,
    /// 目前的 IP
    current_ip: RwLock<String>,
    /// 股票即時報價快照快取 (目前主要由 HiStock 驅動)
    pub stock_snapshots: RwLock<HashMap<String, RealtimeSnapshot>>,
}

impl Share {
    /// 建立一個新的 `Share` 實例。
    ///
    /// 此方法會初始化：
    /// - 可變快取容器（空的 `HashMap` + `RwLock`）
    /// - 交易市場對照表（上市/上櫃/興櫃）
    /// - 產業名稱與代碼對照表（含部分別名）
    ///
    /// 此方法不會讀取資料庫，也不會發出 HTTP 請求。
    pub fn new() -> Self {
        Self {
            indices: RwLock::new(HashMap::new()),
            stocks: RwLock::new(HashMap::new()),
            last_revenues: RwLock::new(HashMap::new()),
            last_trading_day_quotes: RwLock::new(HashMap::new()),
            quote_history_records: RwLock::new(HashMap::new()),
            industries: default_industries(),
            exchange_markets: default_exchange_markets(),
            current_ip: RwLock::new(String::new()),
            stock_snapshots: RwLock::new(HashMap::new()),
        }
    }

    /// 以新抓到的完整指數清單覆蓋舊快取。
    fn replace_indices_cache(&self, indices: Vec<index::Index>) {
        let mut new_cache = HashMap::with_capacity(indices.len());
        for index in indices {
            new_cache.insert(index.key(), index);
        }

        match self.indices.write() {
            Ok(mut cache) => *cache = new_cache,
            Err(why) => {
                logging::error_file_async(format!(
                    "Failed to replace indices cache because {:?}",
                    why
                ));
            }
        }
    }

    /// 以新抓到的完整股票主檔清單覆蓋舊快取。
    fn replace_stocks_cache(&self, stocks: Vec<stock::Stock>) {
        let mut new_cache = HashMap::with_capacity(stocks.len());
        for stock in stocks {
            new_cache.insert(stock.stock_symbol.to_string(), stock);
        }

        match self.stocks.write() {
            Ok(mut cache) => *cache = new_cache,
            Err(why) => {
                logging::error_file_async(format!(
                    "Failed to replace stocks cache because {:?}",
                    why
                ));
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
                logging::error_file_async(format!(
                    "Failed to replace last_revenues cache because {:?}",
                    why
                ));
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
                logging::error_file_async(format!(
                    "Failed to replace last_trading_day_quotes cache because {:?}",
                    why
                ));
            }
        }
    }

    /// 以新抓到的歷史高低紀錄清單覆蓋舊快取。
    fn replace_quote_history_records_cache(
        &self,
        records: Vec<quote_history_record::QuoteHistoryRecord>,
    ) {
        let mut new_cache = HashMap::with_capacity(records.len());
        for record in records {
            new_cache.insert(record.security_code.to_string(), record);
        }

        match self.quote_history_records.write() {
            Ok(mut cache) => *cache = new_cache,
            Err(why) => {
                logging::error_file_async(format!(
                    "Failed to replace quote_history_records cache because {:?}",
                    why
                ));
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
        match index::Index::fetch().await {
            Ok(indices) => self.replace_indices_cache(indices),
            Err(why) => {
                logging::error_file_async(format!("Failed to fetch indices because {:?}", why));
            }
        }

        match stock::Stock::fetch().await {
            Ok(stocks) => self.replace_stocks_cache(stocks),
            Err(why) => {
                logging::error_file_async(format!("Failed to fetch stocks because {:?}", why));
            }
        }

        match revenue::fetch_last_two_month().await {
            Ok(revenues) => self.replace_last_revenues_cache(revenues),
            Err(why) => {
                logging::error_file_async(format!(
                    "Failed to fetch last_revenues because {:?}",
                    why
                ));
            }
        }

        match last_daily_quotes::LastDailyQuotes::fetch().await {
            Ok(quotes) => self.replace_last_trading_day_quotes_cache(quotes),
            Err(why) => {
                logging::error_file_async(format!(
                    "Failed to fetch last_trading_day_quotes because {:?}",
                    why
                ));
            }
        }

        match quote_history_record::QuoteHistoryRecord::fetch().await {
            Ok(records) => self.replace_quote_history_records_cache(records),
            Err(why) => {
                logging::error_file_async(format!(
                    "Failed to fetch quote_history_records because {:?}",
                    why
                ));
            }
        }

        if let Ok(ip) = crawler_share::get_public_ip().await {
            self.set_current_ip(ip);
        }

        let current_ip = self.get_current_ip().unwrap_or_default();
        logging::info_file_async(format!("current_ip  {}", current_ip));
        logging::info_file_async(format!(
            "CacheShare.indices 初始化 {}",
            self.indices
                .read()
                .map(|cache| cache.len())
                .unwrap_or_default()
        ));
        logging::info_file_async(format!(
            "CacheShare.industries 初始化 {:?}",
            self.industries
        ));
        logging::info_file_async(format!(
            "CacheShare.stocks 初始化 {}",
            self.stocks
                .read()
                .map(|cache| cache.len())
                .unwrap_or_default()
        ));
        logging::info_file_async(format!(
            "CacheShare.last_trading_day_quotes 初始化 {}",
            self.last_trading_day_quotes
                .read()
                .map(|cache| cache.len())
                .unwrap_or_default()
        ));
        logging::info_file_async(format!(
            "CacheShare.quote_history_records 初始化 {}",
            self.quote_history_records
                .read()
                .map(|cache| cache.len())
                .unwrap_or_default()
        ));

        if let Ok(revenues) = self.last_revenues.read() {
            for revenue in revenues.iter() {
                logging::info_file_async(format!(
                    "CacheShare.last_revenues 初始化 {}:{}",
                    revenue.0,
                    revenue.1.keys().len()
                ));
            }
        }
    }

    /// 更新目前對外 IP 到快取。
    ///
    /// # 參數
    /// - `ip`: 目前偵測到的對外 IP。
    ///
    /// # 行為
    /// - 若寫入鎖成功，會直接覆蓋舊值。
    /// - 若寫入鎖失敗，方法會靜默略過，不會 panic。
    pub fn set_current_ip(&self, ip: String) {
        if let Ok(mut current_ip) = self.current_ip.write() {
            *current_ip = ip;
        }
    }

    /// 從快取取得目前對外 IP。
    ///
    /// # 回傳
    /// - `Some(String)`：成功讀取，目前值可能是空字串。
    /// - `None`：讀鎖失敗。
    pub fn get_current_ip(&self) -> Option<String> {
        match self.current_ip.read() {
            Ok(ip) => Some(ip.clone()),
            Err(_) => None,
        }
    }

    /// 寫入或覆蓋單筆台股指數快取。
    ///
    /// # 參數
    /// - `key`: 指數快取鍵值。
    /// - `index`: 欲寫入的指數資料。
    ///
    /// # 回傳
    /// - `Some(old_value)`：原本已有資料，回傳被覆蓋的舊值。
    /// - `None`：原本沒有資料。
    /// - `Some(index)`：若寫入鎖失敗，回傳原輸入值，讓呼叫端可自行決定是否重試。
    pub async fn set_stock_index(&self, key: String, index: index::Index) -> Option<index::Index> {
        match self.indices.write() {
            Ok(mut indices) => indices.insert(key, index),
            Err(_) => Some(index),
        }
    }

    /// 依鍵值讀取台股指數快取。
    ///
    /// # 參數
    /// - `key`: 指數快取鍵值。
    ///
    /// # 回傳
    /// - `Some(index::Index)`：找到資料。
    /// - `None`：未命中或讀鎖失敗。
    pub fn get_stock_index(&self, key: &str) -> Option<index::Index> {
        match self.indices.read() {
            Ok(cache) => cache.get(key).cloned(),
            Err(_) => None,
        }
    }

    /// 依交易市場代碼取得市場描述資料。
    ///
    /// # 參數
    /// - `id`: 交易市場代碼，例如上市 `2`、上櫃 `4`、興櫃 `5`。
    ///
    /// # 回傳
    /// - `Some(StockExchangeMarket)`：找到對應市場。
    /// - `None`：查無此代碼。
    pub fn get_exchange_market(
        &self,
        id: i32,
    ) -> Option<stock_exchange_market::StockExchangeMarket> {
        self.exchange_markets.get(&id).cloned()
    }

    /// 透過產業名稱取得對應的產業代碼。
    ///
    /// # 參數
    /// - `name`: 產業中文名稱。
    ///
    /// # 回傳
    /// - `Some(code)`：找到對應代碼。
    /// - `Some(99)`：未命中時回傳預設「未分類」。
    ///
    /// 此方法保留 `Option` 型別，是為了與其他快取查詢 API 維持一致。
    pub fn get_industry_id(&self, name: &str) -> Option<i32> {
        match self.industries.get(name) {
            None => Some(99),
            Some(industry) => Some(*industry),
        }
    }

    /// 透過產業代碼反查第一個符合的產業名稱。
    ///
    /// # 參數
    /// - `id`: 產業代碼。
    ///
    /// # 回傳
    /// - `Some(String)`：找到符合的產業名稱。
    /// - `None`：查無對應代碼。
    ///
    /// # 注意
    /// 因 `industries` 內包含少量同義名稱，若同一代碼對應多個名稱，
    /// 本方法只會回傳迭代過程遇到的第一筆。
    pub fn get_industry_name(&self, id: i32) -> Option<String> {
        self.industries.iter().find_map(|(key, &value)| {
            if value == id {
                Some(key.to_string())
            } else {
                None
            }
        })
    }

    /// 依股票代號讀取股票主檔快取。
    ///
    /// # 參數
    /// - `symbol`: 股票代號。
    ///
    /// # 回傳
    /// - `Some(stock::Stock)`：找到資料。
    /// - `None`：未命中或讀鎖失敗。
    pub async fn get_stock(&self, symbol: &str) -> Option<stock::Stock> {
        match self.stocks.read() {
            Ok(cache) => cache.get(symbol).cloned(),
            Err(_) => None,
        }
    }

    /// 判斷股票主檔快取是否包含指定股票代號。
    ///
    /// # 參數
    /// - `symbol`: 股票代號。
    ///
    /// # 回傳
    /// - `true`：快取存在該股票。
    /// - `false`：不存在，或讀鎖失敗。
    pub fn stock_contains_key(&self, symbol: &str) -> bool {
        match self.stocks.read() {
            Ok(cache) => cache.contains_key(symbol),
            Err(_) => false,
        }
    }

    /// 取得某檔股票的「最後交易日報價」快取資料。
    ///
    /// # 參數
    /// - `symbol`: 股票代號。
    ///
    /// # 回傳
    /// - `Some(LastDailyQuotes)`：找到資料。
    /// - `None`：未命中或讀鎖失敗。
    pub async fn get_stock_last_price(
        &self,
        symbol: &str,
    ) -> Option<last_daily_quotes::LastDailyQuotes> {
        match self.last_trading_day_quotes.read() {
            Ok(cache) => cache.get(symbol).cloned(),
            Err(_) => None,
        }
    }

    /// 將單筆月營收資料寫入 `last_revenues` 快取。
    ///
    /// # 參數
    /// - `revenue`: 欲寫入的月營收資料。
    ///
    /// # 行為
    /// - 若該月份 bucket 尚未存在，會自動建立。
    /// - 同月同股號若已有資料，保留原值，不覆蓋舊值。
    /// - 若寫入鎖失敗，會直接略過。
    pub fn set_last_revenues(&self, revenue: revenue::Revenue) {
        if let Ok(mut last_revenues) = self.last_revenues.write() {
            last_revenues
                .entry(revenue.date)
                .or_insert_with(HashMap::new)
                .entry(revenue.stock_symbol.to_string())
                .or_insert(revenue);
        }
    }

    /// 檢查 `last_revenues` 是否存在指定月份與股票代號的資料。
    ///
    /// # 參數
    /// - `key1`: 月份鍵值，格式通常為 `yyyyMM`。
    /// - `key2`: 股票代號。
    ///
    /// # 回傳
    /// - `true`：指定月份下存在該股票資料。
    /// - `false`：不存在，或讀鎖失敗。
    pub fn last_revenues_contains_key(&self, key1: i64, key2: &str) -> bool {
        self.last_revenues
            .read()
            .map(|cache| {
                cache
                    .get(&key1)
                    .is_some_and(|last_revenue| last_revenue.contains_key(key2))
            })
            .unwrap_or(false)
    }

    /// 更新最後交易日報價快取中的既有股票資料。
    ///
    /// # 參數
    /// - `daily_quote`: 新的日行情資料。
    ///
    /// # 行為
    /// - 僅更新已存在於快取中的股票。
    /// - 目前只同步 `date` 與 `closing_price`。
    /// - 若快取內沒有該股票，方法不會新增資料。
    pub async fn set_stock_last_price(&self, daily_quote: &daily_quote::DailyQuote) {
        if let Ok(mut last_trading_day_quotes) = self.last_trading_day_quotes.write() {
            if let Some(quote) = last_trading_day_quotes.get_mut(&daily_quote.stock_symbol) {
                quote.date = daily_quote.date;
                quote.closing_price = daily_quote.closing_price;
            }
        }
    }

    /// 取得最後交易日報價快取資料（與 [`Self::get_stock_last_price`] 等價）。
    ///
    /// # 參數
    /// - `symbol`: 股票代號。
    ///
    /// # 回傳
    /// - `Some(LastDailyQuotes)`：找到資料。
    /// - `None`：未命中或讀鎖失敗。
    pub async fn get_last_trading_day_quotes(
        &self,
        symbol: &str,
    ) -> Option<last_daily_quotes::LastDailyQuotes> {
        match self.last_trading_day_quotes.read() {
            Ok(cache) => cache.get(symbol).cloned(),
            Err(_) => None,
        }
    }

    /// 將股票全市場報價快照寫入快取。
    ///
    /// # 參數
    /// - `snapshots`: 以股票代號為 key 的全量即時快照。
    ///
    /// # 行為
    /// - 會直接以新資料覆蓋整個快照快取。
    /// - 適合由全量抓取器在單次更新完成後整批替換。
    pub fn set_stock_snapshots(&self, snapshots: HashMap<String, RealtimeSnapshot>) {
        if let Ok(mut cache) = self.stock_snapshots.write() {
            *cache = snapshots;
        }
    }

    /// 寫入或更新單筆股票報價快照中的最新成交價。
    ///
    /// # 參數
    /// - `symbol`: 股票代號。
    /// - `price`: 最新成交價。
    ///
    /// # 行為
    /// - 若快取內已存在該股票，僅更新 `price` 欄位，保留名稱、漲跌幅、
    ///   開高低與成交量等其他欄位。
    /// - 若快取內尚無該股票，會建立一筆只含必要欄位的最小快照。
    /// - 適合用於「單檔備援更新」情境，避免用不完整資料覆蓋整筆快照。
    pub fn set_stock_snapshot_price(&self, symbol: String, price: Decimal) {
        if let Ok(mut cache) = self.stock_snapshots.write() {
            if let Some(snapshot) = cache.get_mut(&symbol) {
                snapshot.price = price;
            } else {
                cache.insert(symbol.clone(), RealtimeSnapshot::new(symbol, price));
            }
        }
    }

    /// 從快取取得股票報價快照。
    ///
    /// # 參數
    /// - `symbol`: 股票代號。
    ///
    /// # 回傳
    /// - `Some(RealtimeSnapshot)`：找到資料。
    /// - `None`：未命中或讀鎖失敗。
    pub fn get_stock_snapshot(&self, symbol: &str) -> Option<RealtimeSnapshot> {
        self.stock_snapshots
            .read()
            .ok()
            .and_then(|cache| cache.get(symbol).cloned())
    }

    /// 清空股票報價快照快取。
    ///
    /// 這通常用於即時報價任務停止、收盤後釋放記憶體，
    /// 或需要強制失效目前全量快照時。
    pub fn clear_stock_snapshots(&self) {
        if let Ok(mut cache) = self.stock_snapshots.write() {
            *cache = HashMap::new();
        }
    }
}

impl Default for Share {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use rust_decimal::Decimal;

    use super::*;

    /// 建立測試用營收資料。
    fn make_test_revenue(stock_symbol: &str, date: i64) -> revenue::Revenue {
        let mut revenue = revenue::Revenue::new();
        revenue.stock_symbol = stock_symbol.to_string();
        revenue.date = date;
        revenue
    }

    /// 建立測試用指數資料。
    fn make_test_index(category: &str, date: NaiveDate) -> index::Index {
        let mut index = index::Index::new();
        index.category = category.to_string();
        index.date = date;
        index
    }

    /// 驗證新增營收時會自動建立月份 bucket。
    #[test]
    fn test_set_last_revenues_creates_new_month_bucket() {
        let share = Share::new();

        share.set_last_revenues(make_test_revenue("2330", 202501));

        assert!(share.last_revenues_contains_key(202501, "2330"));
    }

    /// 驗證整批覆蓋指數快取會移除舊資料。
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

    /// 驗證單筆更新股價時會保留其他欄位。
    #[test]
    fn test_set_stock_snapshot_price_preserves_existing_fields() {
        let share = Share::new();
        let mut snapshot = RealtimeSnapshot::new("2330".to_string(), Decimal::new(998, 0));
        snapshot.name = "台積電".to_string();
        snapshot.change = Decimal::new(5, 0);

        let mut snapshots = HashMap::new();
        snapshots.insert("2330".to_string(), snapshot);
        share.set_stock_snapshots(snapshots);

        share.set_stock_snapshot_price("2330".to_string(), Decimal::new(1000, 0));

        let updated = share.get_stock_snapshot("2330").unwrap();
        assert_eq!(updated.price, Decimal::new(1000, 0));
        assert_eq!(updated.name, "台積電");
        assert_eq!(updated.change, Decimal::new(5, 0));
    }

    /// 驗證整批覆蓋營收快取會淘汰舊月份資料。
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

    /// 驗證產業代碼可反查名稱。
    #[tokio::test]
    async fn test_get_industry_name() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        assert_eq!(SHARE.get_industry_name(1), Some("水泥工業".to_string()));
        assert_eq!(SHARE.get_industry_name(2), Some("食品工業".to_string()));
        assert_eq!(SHARE.get_industry_name(99), Some("未分類".to_string()));
        assert_eq!(SHARE.get_industry_name(100), None);
    }

    /// 驗證主快取載入流程。
    #[tokio::test]
    async fn test_load() {
        dotenv::dotenv().ok();

        SHARE.load().await;

        let mut loop_count = 10;
        for e in SHARE.indices.read().unwrap().iter() {
            if loop_count < 0 {
                break;
            }

            logging::info_file_async(format!(
                "indices e.date {:?} e.index {:?}",
                e.1.date, e.1.index
            ));

            loop_count -= 1;
        }

        loop_count = 10;
        for (k, v) in SHARE.stocks.read().unwrap().iter() {
            if loop_count < 0 {
                break;
            }

            logging::info_file_async(format!("stock {} name {}", k, v.name));
            loop_count -= 1;
        }

        loop_count = 10;
        for (k, v) in SHARE.last_trading_day_quotes.read().unwrap().iter() {
            if loop_count < 0 {
                break;
            }

            logging::info_file_async(format!(
                "security_code {} closing_price {}",
                k, v.closing_price
            ));
            loop_count -= 1;
        }

        for (k, v) in SHARE.industries.iter() {
            logging::info_file_async(format!("name {}  category {}", k, v));
        }

        match SHARE.quote_history_records.write() {
            Ok(mut quote_history_records_guard) => {
                if let Some(qhr) = quote_history_records_guard.get_mut("2330") {
                    qhr.minimum_price = Decimal::from(1);
                    qhr.maximum_price = Decimal::from(2);
                }
            }
            Err(_) => todo!(),
        }

        for (k, v) in SHARE.quote_history_records.read().unwrap().iter() {
            if k == "2330" {
                dbg!(v);
            }
        }
    }
}
