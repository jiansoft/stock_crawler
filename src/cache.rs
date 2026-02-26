//! 全域快取模組。
//!
//! 本模組提供兩類快取：
//! 1. [`SHARE`]：長生命週期的業務資料快取，包含股票主檔、產業分類、指數、
//!    最近月營收、最後交易日收盤價與歷史高低統計。
//! 2. [`TTL`]：短生命週期的暫存快取，適合「短時間內避免重複處理」的場景。
//!
//! 設計上以 `RwLock` 保護共享資料，讀多寫少的路徑可並行讀取。
//! 若鎖取得失敗，多數 API 會回傳 `None` 或 `false` 以避免 panic，
//! 並由上層依回傳值決定是否重試或降級處理。

use std::{collections::HashMap, sync::RwLock, time::Duration};

use once_cell::sync::Lazy;
use rust_decimal::Decimal;

//use futures::executor::block_on;

use crate::crawler::share;
use crate::{
    database::table::{
        daily_quote, index, last_daily_quotes, quote_history_record, revenue, stock,
        stock_exchange_market,
    },
    declare::{self, Industry},
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
    // quote_history_records 股票歷史、淨值比等最高、最低的數據,resource.Init() 從資料庫內讀取出，若抓到新的數據時則會同時更新資料庫與此數據
    pub quote_history_records: RwLock<HashMap<String, quote_history_record::QuoteHistoryRecord>>,
    /// 股票產業分類
    industries: HashMap<String, i32>,
    /// 股票產業分類(2, 'TAI', '上市', 1),(4, 'TWO', '上櫃', 2), (5, 'TWE', '興櫃', 2);
    exchange_markets: HashMap<i32, stock_exchange_market::StockExchangeMarket>,
    /// 目前的 IP
    current_ip: RwLock<String>,
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
        // let other : &'static str = format!("{}業", Industry::Other.name());
        //// &'static str
        Share {
            indices: RwLock::new(HashMap::new()),
            stocks: RwLock::new(HashMap::new()),
            exchange_markets: HashMap::from([
                (
                    2,
                    stock_exchange_market::StockExchangeMarket {
                        stock_exchange_market_id: 2,
                        stock_exchange_id: 1,
                        code: "TAI".to_string(),
                        name: declare::StockExchangeMarket::Listed.name(),
                    },
                ),
                (
                    4,
                    stock_exchange_market::StockExchangeMarket {
                        stock_exchange_market_id: 4,
                        stock_exchange_id: 2,
                        code: "TWO".to_string(),
                        name: declare::StockExchangeMarket::OverTheCounter.name(),
                    },
                ),
                (
                    5,
                    stock_exchange_market::StockExchangeMarket {
                        stock_exchange_market_id: 5,
                        stock_exchange_id: 2,
                        code: "TWE".to_string(),
                        name: declare::StockExchangeMarket::Emerging.name(),
                    },
                ),
            ]),
            industries: HashMap::from([
                (Industry::Cement.name(), Industry::Cement.serial()),
                (Industry::Food.name(), Industry::Food.serial()),
                (Industry::Plastic.name(), Industry::Plastic.serial()),
                (
                    Industry::TextileFiber.name(),
                    Industry::TextileFiber.serial(),
                ),
                (
                    Industry::ElectricalMachinery.name(),
                    Industry::ElectricalMachinery.serial(),
                ),
                (
                    Industry::ElectricalCable.name(),
                    Industry::ElectricalCable.serial(),
                ),
                (Industry::Chemical.name(), Industry::Chemical.serial()),
                (
                    Industry::BiotechMedical.name(),
                    Industry::BiotechMedical.serial(),
                ),
                (
                    Industry::GlassCeramics.name(),
                    Industry::GlassCeramics.serial(),
                ),
                (Industry::Paper.name(), Industry::Paper.serial()),
                (Industry::Steel.name(), Industry::Steel.serial()),
                (Industry::Rubber.name(), Industry::Rubber.serial()),
                (Industry::Automotive.name(), Industry::Automotive.serial()),
                (
                    Industry::Semiconductor.name(),
                    Industry::Semiconductor.serial(),
                ),
                (
                    Industry::ComputerPeripheral.name(),
                    Industry::ComputerPeripheral.serial(),
                ),
                (
                    Industry::Optoelectronic.name(),
                    Industry::Optoelectronic.serial(),
                ),
                (
                    Industry::CommunicationNetwork.name(),
                    Industry::CommunicationNetwork.serial(),
                ),
                (
                    Industry::ElectronicComponents.name(),
                    Industry::ElectronicComponents.serial(),
                ),
                (
                    Industry::ElectronicPathway.name(),
                    Industry::ElectronicPathway.serial(),
                ),
                (
                    Industry::InformationService.name(),
                    Industry::InformationService.serial(),
                ),
                (
                    Industry::OtherElectronics.name(),
                    Industry::OtherElectronics.serial(),
                ),
                (
                    Industry::ConstructionMaterial.name(),
                    Industry::ConstructionMaterial.serial(),
                ),
                (Industry::Shipping.name(), Industry::Shipping.serial()),
                (
                    Industry::FinanceInsurance.name(),
                    Industry::FinanceInsurance.serial(),
                ),
                (
                    Industry::TradingDepartmentStores.name(),
                    Industry::TradingDepartmentStores.serial(),
                ),
                (
                    Industry::OilElectricGas.name(),
                    Industry::OilElectricGas.serial(),
                ),
                (
                    Industry::Comprehensive.name(),
                    Industry::Comprehensive.serial(),
                ),
                (
                    Industry::GreenEnergyEnvironmentalProtection.name(),
                    Industry::GreenEnergyEnvironmentalProtection.serial(),
                ),
                (
                    Industry::DigitalCloud.name(),
                    Industry::DigitalCloud.serial(),
                ),
                (
                    Industry::SportsRecreation.name(),
                    Industry::SportsRecreation.serial(),
                ),
                (Industry::HomeLife.name(), Industry::HomeLife.serial()),
                (Industry::Other.name(), Industry::Other.serial()),
                (
                    Industry::CulturalCreative.name(),
                    Industry::CulturalCreative.serial(),
                ),
                (
                    Industry::AgriculturalTechnology.name(),
                    Industry::AgriculturalTechnology.serial(),
                ),
                (Industry::ECommerce.name(), Industry::ECommerce.serial()),
                (
                    Industry::TourismCatering.name(),
                    Industry::TourismCatering.serial(),
                ),
                (
                    Industry::DepositaryReceipts.name(),
                    Industry::DepositaryReceipts.serial(),
                ),
                (
                    Industry::Uncategorized.name(),
                    Industry::Uncategorized.serial(),
                ),
                (
                    "貿易百貨業".to_string(),
                    Industry::TradingDepartmentStores.serial(),
                ),
                ("其他業".to_string(), Industry::Other.serial()),
                (
                    "農業科技業".to_string(),
                    Industry::AgriculturalTechnology.serial(),
                ),
            ]),
            last_revenues: RwLock::new(HashMap::new()),
            last_trading_day_quotes: RwLock::new(HashMap::new()),
            quote_history_records: RwLock::new(HashMap::new()),
            current_ip: RwLock::new(String::new()),
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
    /// - 方法本身不回傳 `Result`，屬於「盡力載入」模型。
    ///
    /// 建議在程式啟動階段呼叫一次；若需要定期刷新，可由排程層控制呼叫時機。
    pub async fn load(&self) {
        let indices = index::Index::fetch().await;
        match self.indices.write() {
            Ok(mut i) => {
                if let Ok(indices) = indices {
                    for index in indices {
                        i.insert(index.key(), index);
                    }
                }
            }
            Err(why) => {
                logging::error_file_async(format!("Failed to indices.write because {:?}", why));
            }
        }

        let stocks = stock::Stock::fetch().await;
        match self.stocks.write() {
            Ok(mut s) => {
                if let Ok(result) = stocks {
                    for e in result {
                        s.insert(e.stock_symbol.to_string(), e);
                    }
                }
            }
            Err(why) => {
                logging::error_file_async(format!("Failed to stocks.write because {:?}", why));
            }
        }

        if let (Ok(result), Ok(mut last_revenue)) = (
            revenue::fetch_last_two_month().await,
            self.last_revenues.write(),
        ) {
            result.iter().for_each(|e| {
                last_revenue
                    .entry(e.date)
                    .or_insert_with(HashMap::new)
                    .insert(e.stock_symbol.to_string(), e.clone());
            });
        } else {
            logging::error_file_async("Failed to update last_revenues".to_string());
        }

        let last_daily_quotes = last_daily_quotes::LastDailyQuotes::fetch().await;
        if let (Ok(result), Ok(mut ldq)) =
            (&last_daily_quotes, self.last_trading_day_quotes.write())
        {
            for e in result {
                ldq.insert(e.stock_symbol.to_string(), e.clone());
            }
        } else {
            logging::error_file_async(format!(
                "Failed to update last_trading_day_quotes: {:?}",
                last_daily_quotes.err()
            ));
        }

        let quote_history_records = quote_history_record::QuoteHistoryRecord::fetch().await;
        match self.quote_history_records.write() {
            Ok(mut s) => {
                if let Ok(result) = quote_history_records {
                    for e in result {
                        s.insert(e.security_code.to_string(), e);
                    }
                }
            }
            Err(why) => {
                logging::error_file_async(format!(
                    "Failed to quote_history_records.write because {:?}",
                    why
                ));
            }
        }

        if let Ok(ip) = share::get_public_ip().await {
            self.set_current_ip(ip);
        }

        logging::info_file_async(format!("current_ip  {}", self.current_ip.read().unwrap()));

        logging::info_file_async(format!(
            "CacheShare.indices 初始化 {}",
            self.indices.read().unwrap().len()
        ));

        logging::info_file_async(format!(
            "CacheShare.industries 初始化 {:?}",
            self.industries
        ));

        logging::info_file_async(format!(
            "CacheShare.stocks 初始化 {}",
            self.stocks.read().unwrap().len()
        ));

        logging::info_file_async(format!(
            "CacheShare.last_trading_day_quotes 初始化 {}",
            self.last_trading_day_quotes.read().unwrap().len()
        ));
        logging::info_file_async(format!(
            "CacheShare.quote_history_records 初始化 {}",
            self.quote_history_records.read().unwrap().len()
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
    /// - `ip`: 最新的 IP 字串。
    ///
    /// 若寫入鎖取得失敗，本方法會直接略過，不會 panic。
    pub fn set_current_ip(&self, ip: String) {
        if let Ok(mut current_ip) = self.current_ip.write() {
            *current_ip = ip;
        }
    }

    /// 從快取取得目前對外 IP。
    ///
    /// # 回傳
    /// - `Some(String)`: 讀取成功（可能為空字串，代表尚未設定）。
    /// - `None`: 讀取鎖失敗。
    pub fn get_current_ip(&self) -> Option<String> {
        match self.current_ip.read() {
            Ok(ip) => Some(ip.clone()),
            Err(_) => None,
        }
    }

    /// 寫入或覆蓋單筆台股指數快取。
    ///
    /// # 參數
    /// - `key`: 指數鍵值（通常是日期或市場識別字串）。
    /// - `index`: 指數資料實體。
    ///
    /// # 回傳
    /// - `Some(old_value)`: 原本已有資料，回傳被覆蓋的舊值。
    /// - `None`: 原本沒有資料，完成新增。
    /// - `Some(index)`: 若寫入鎖失敗，回傳原輸入值，讓呼叫端可選擇重試。
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
    /// - `Some(index::Index)`: 找到資料。
    /// - `None`: 未找到或讀鎖失敗。
    pub fn get_stock_index(&self, key: &str) -> Option<index::Index> {
        match self.indices.read() {
            Ok(cache) => cache.get(key).cloned(),
            Err(_) => None,
        }
    }

    /// 依交易市場代碼取得市場描述資料。
    ///
    /// # 參數
    /// - `id`: 市場代碼，例如 2/4/5（上市/上櫃/興櫃）。
    ///
    /// # 回傳
    /// - `Some(StockExchangeMarket)`: 成功對應。
    /// - `None`: 無此代碼。
    pub fn get_exchange_market(
        &self,
        id: i32,
    ) -> Option<stock_exchange_market::StockExchangeMarket> {
        SHARE.exchange_markets.get(&id).cloned()
    }

    /// 透過產業名稱取得對應的產業代碼。
    ///
    /// # 參數
    /// - `name`: 產業中文名稱。
    ///
    /// # 回傳
    /// - `Some(code)`: 找到對應代碼。
    /// - `Some(99)`: 未命中時回傳預設「未分類」代碼。
    ///
    /// 註：目前實作不會回傳 `None`，回傳型別保留 `Option` 是為了與其他查詢 API 介面一致。
    pub fn get_industry_id(&self, name: &str) -> Option<i32> {
        // 如果找到了行業，則返回相應的ID。如果沒有找到，則返回99。
        match SHARE.industries.get(name) {
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
    /// - `Some(String)`: 找到第一個符合的名稱。
    /// - `None`: 無對應代碼。
    ///
    /// 註：因目前 `industries` 內含少量同義名稱，若代碼對應多個名稱，
    /// 本方法只會回傳迭代到的第一筆。
    pub fn get_industry_name(&self, id: i32) -> Option<String> {
        let result = self.industries.iter().find_map(|(key, &value)| {
            if value == id {
                Some(key.to_string())
            } else {
                None
            }
        });
        result
    }

    /// 依股票代號讀取股票主檔快取。
    ///
    /// # 參數
    /// - `symbol`: 股票代號。
    ///
    /// # 回傳
    /// - `Some(stock::Stock)`: 找到資料。
    /// - `None`: 未找到或讀鎖失敗。
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
    /// - `true`: 快取內存在該代號。
    /// - `false`: 不存在，或讀鎖失敗。
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
    /// - `Some(LastDailyQuotes)`: 找到資料。
    /// - `None`: 未找到或讀鎖失敗。
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
    /// - `revenue`: 欲寫入的月營收資料，會使用其 `date` 與 `stock_symbol` 作為索引。
    ///
    /// # 行為細節
    /// - 僅在「該月份 key 已存在」時才會嘗試插入。
    /// - 同月同股號若已存在資料，維持原值，不覆蓋舊值。
    /// - 若寫入鎖失敗或月份 bucket 不存在，方法會直接略過。
    pub fn set_last_revenues(&self, revenue: revenue::Revenue) {
        if let Ok(mut last_revenues) = SHARE.last_revenues.write() {
            if let Some(last_revenue_date) = last_revenues.get_mut(&revenue.date) {
                last_revenue_date
                    .entry(revenue.stock_symbol.to_string())
                    .or_insert(revenue.clone());
            }
        }
    }

    /// 檢查 `last_revenues` 是否存在指定月份與股票代號的資料。
    ///
    /// # 參數
    /// - `key1`: 月份鍵值（`yyyyMM`）。
    /// - `key2`: 股票代號。
    ///
    /// # 回傳
    /// - `true`: 在指定月份中找到該股票代號。
    /// - `false`: 未找到，或讀鎖失敗。
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
    /// - `daily_quote`: 新進來的日行情資料。
    ///
    /// # 行為細節
    /// - 只更新已存在於快取中的股票。
    /// - 目前僅同步 `date` 與 `closing_price` 欄位。
    /// - 若目標股票不存在或寫入鎖失敗，不會新增資料也不會報錯。
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
    /// - `Some(LastDailyQuotes)`: 找到資料。
    /// - `None`: 未找到或讀鎖失敗。
    ///
    /// 此方法保留主要是為了呼叫端語意可讀性，實作上與 `get_stock_last_price` 相同。
    pub async fn get_last_trading_day_quotes(
        &self,
        symbol: &str,
    ) -> Option<last_daily_quotes::LastDailyQuotes> {
        match self.last_trading_day_quotes.read() {
            Ok(cache) => cache.get(symbol).cloned(),
            Err(_) => None,
        }
    }
}

impl Default for Share {
    fn default() -> Self {
        Self::new()
    }
}

/// 全域短時效快取實例。
///
/// 適合儲存短時間內需要去重、節流或通知判斷的資料。
pub static TTL: Lazy<Ttl> = Lazy::new(Default::default);

/// 具 TTL（存活時間）能力的快取容器。
///
/// 目前包含兩類資料：
/// - `daily_quote`：避免同一輪流程重複處理同一筆日行情。
/// - `trace_quote_notify`：記錄通知相關狀態，避免短時間重複通知。
pub struct Ttl {
    /// 每日收盤數據
    daily_quote: RwLock<ttl_cache::TtlCache<String, String>>,
    trace_quote_notify: RwLock<ttl_cache::TtlCache<String, Decimal>>,
}

/// 對 `Ttl` 的操作介面抽象。
///
/// 這個 trait 讓呼叫端可以透過一致 API 操作不同 TTL 區塊，
/// 並把鎖失敗時的降級行為（`None`/`false`）統一封裝在實作層。
pub trait TtlCacheInner {
    /// 清空 `daily_quote` 區塊。
    fn clear(&self);
    /// 檢查 `daily_quote` 是否包含指定 key。
    fn daily_quote_contains_key(&self, key: &str) -> bool;
    /// 讀取 `daily_quote` 的值。
    fn daily_quote_get(&self, key: &str) -> Option<String>;
    /// 寫入 `daily_quote`，並設定存活時間。
    fn daily_quote_set(
        &self,
        key: String,
        val: String,
        duration: std::time::Duration,
    ) -> Option<String>;
    /// 檢查 `trace_quote_notify` 是否包含指定 key。
    fn trace_quote_contains_key(&self, key: &str) -> bool;
    /// 讀取 `trace_quote_notify` 的值。
    fn trace_quote_get(&self, key: &str) -> Option<Decimal>;
    /// 寫入 `trace_quote_notify`，並設定存活時間。
    fn trace_quote_set(&self, key: String, val: Decimal, duration: Duration) -> Option<Decimal>;
}

impl TtlCacheInner for Ttl {
    fn clear(&self) {
        if let Ok(mut ttl) = self.daily_quote.write() {
            ttl.clear()
        }
    }

    fn daily_quote_contains_key(&self, key: &str) -> bool {
        match self.daily_quote.read() {
            Ok(ttl) => ttl.contains_key(key),
            Err(_) => false,
        }
    }

    fn daily_quote_get(&self, key: &str) -> Option<String> {
        match self.daily_quote.read() {
            Ok(ttl) => ttl.get(key).map(|value| value.to_string()),
            Err(_) => None,
        }
    }

    fn daily_quote_set(&self, key: String, val: String, duration: Duration) -> Option<String> {
        match self.daily_quote.write() {
            Ok(mut ttl) => ttl.insert(key, val, duration),
            Err(_) => None,
        }
    }

    fn trace_quote_contains_key(&self, key: &str) -> bool {
        match self.trace_quote_notify.read() {
            Ok(ttl) => ttl.contains_key(key),
            Err(_) => false,
        }
    }

    fn trace_quote_get(&self, key: &str) -> Option<Decimal> {
        match self.trace_quote_notify.read() {
            Ok(ttl) => ttl.get(key).copied(),
            Err(_) => None,
        }
    }
    fn trace_quote_set(&self, key: String, val: Decimal, duration: Duration) -> Option<Decimal> {
        match self.trace_quote_notify.write() {
            Ok(mut ttl) => ttl.insert(key, val, duration),
            Err(_) => None,
        }
    }
}

impl Ttl {
    /// 建立新的 `Ttl` 容器並配置各區塊初始容量。
    ///
    /// 容量規劃：
    /// - `daily_quote`: 2048
    /// - `trace_quote_notify`: 128
    ///
    /// 這些容量只影響初始配置，不代表固定上限。
    pub fn new() -> Self {
        Ttl {
            daily_quote: RwLock::new(ttl_cache::TtlCache::new(2048)),
            trace_quote_notify: RwLock::new(ttl_cache::TtlCache::new(128)),
        }
    }
}

impl Default for Ttl {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rust_decimal::Decimal;

    use super::*;

    #[tokio::test]
    async fn test_get_industry_name() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        assert_eq!(SHARE.get_industry_name(1), Some("水泥工業".to_string()));
        assert_eq!(SHARE.get_industry_name(2), Some("食品工業".to_string()));
        assert_eq!(SHARE.get_industry_name(99), Some("未分類".to_string()));
        assert_eq!(SHARE.get_industry_name(100), None);

        println!("36 => {:?}", SHARE.get_industry_name(36));
    }

    #[tokio::test]
    async fn test_init() {
        dotenv::dotenv().ok();
        let _ = SHARE.indices.read().is_ok();

        let duration = Duration::from_millis(500);
        TTL.daily_quote
            .write()
            .unwrap()
            .insert("1".to_string(), "10".to_string(), duration);

        match TTL.daily_quote_get("1") {
            Some(value) => println!("找到緩存項：{}", value),
            None => println!("緩存項不存在"),
        }

        assert_eq!(TTL.daily_quote_get("1"), Some("10".to_string()));
        tokio::time::sleep(Duration::from_secs(1)).await;

        assert_eq!(TTL.daily_quote_get("1"), None);
    }

    macro_rules! aw {
        ($e:expr) => {
            tokio_test::block_on($e)
        };
    }

    #[test]
    fn test_load() {
        dotenv::dotenv().ok();

        aw!(async {
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
                    match quote_history_records_guard.get_mut("2330") {
                        None => {}
                        Some(qhr) => {
                            qhr.minimum_price = Decimal::from(1);
                            qhr.maximum_price = Decimal::from(2);
                        }
                    }
                }
                Err(_) => todo!(),
            }

            for (k, v) in SHARE.quote_history_records.read().unwrap().iter() {
                if k == "2330" {
                    dbg!(v);
                    // logging::debug_file_async(format!("name {}  category {:?}", k, v));
                }
            }
        });
    }
}
