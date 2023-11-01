use std::{collections::HashMap, sync::RwLock, time::Duration};

use once_cell::sync::Lazy;
use rust_decimal::Decimal;

//use futures::executor::block_on;
use crate::{
    database::table::{
        daily_quote, index, last_daily_quotes, quote_history_record, revenue, stock,
        stock_exchange_market,
    },
    declare,
    declare::Industry,
    logging,
};

pub static SHARE: Lazy<Share> = Lazy::new(Default::default);

/// Share 各類快取共享集中處
pub struct Share {
    /// 存放台股歷年指數
    indices: RwLock<HashMap<String, index::Index>>,
    /// 存放台股股票代碼
    pub stocks: RwLock<HashMap<String, stock::Stock>>,
    /// 月營收的快取(防止重複寫入)，第一層 Key:日期 yyyyMM 第二層 Key:股號
    pub last_revenues: RwLock<HashMap<i64, HashMap<String, revenue::Revenue>>>,
    /// 存放最後交易日股票報價數據
    pub last_trading_day_quotes: RwLock<HashMap<String, last_daily_quotes::LastDailyQuotes>>,
    // quote_history_records 股票歷史、淨值比等最高、最低的數據,resource.Init() 從資料庫內讀取出，若抓到新的數據時則會同時更新資料庫與此數據
    pub quote_history_records: RwLock<HashMap<String, quote_history_record::QuoteHistoryRecord>>,
    /// 股票產業分類
    industries: HashMap<&'static str, i32>,
    /// 股票產業分類(2, 'TAI', '上市', 1),(4, 'TWO', '上櫃', 2), (5, 'TWE', '興櫃', 2);
    exchange_markets: HashMap<i32, stock_exchange_market::StockExchangeMarket>,
}

impl Share {
    pub fn new() -> Self {
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
                        name: declare::StockExchangeMarket::Listed.name().to_string(),
                    },
                ),
                (
                    4,
                    stock_exchange_market::StockExchangeMarket {
                        stock_exchange_market_id: 4,
                        stock_exchange_id: 2,
                        code: "TWO".to_string(),
                        name: declare::StockExchangeMarket::OverTheCounter
                            .name()
                            .to_string(),
                    },
                ),
                (
                    5,
                    stock_exchange_market::StockExchangeMarket {
                        stock_exchange_market_id: 5,
                        stock_exchange_id: 2,
                        code: "TWE".to_string(),
                        name: declare::StockExchangeMarket::Emerging.name().to_string(),
                    },
                ),
            ]),
            industries: HashMap::from([
                (
                    Industry::CementIndustry.name(),
                    Industry::CementIndustry.serial(),
                ),
                (
                    Industry::FoodIndustry.name(),
                    Industry::FoodIndustry.serial(),
                ),
                (
                    Industry::PlasticIndustry.name(),
                    Industry::PlasticIndustry.serial(),
                ),
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
                (
                    Industry::ChemicalIndustry.name(),
                    Industry::ChemicalIndustry.serial(),
                ),
                (
                    Industry::BiotechMedical.name(),
                    Industry::BiotechMedical.serial(),
                ),
                (
                    Industry::GlassCeramics.name(),
                    Industry::GlassCeramics.serial(),
                ),
                (
                    Industry::PaperIndustry.name(),
                    Industry::PaperIndustry.serial(),
                ),
                (
                    Industry::SteelIndustry.name(),
                    Industry::SteelIndustry.serial(),
                ),
                (
                    Industry::RubberIndustry.name(),
                    Industry::RubberIndustry.serial(),
                ),
                (
                    Industry::AutomotiveIndustry.name(),
                    Industry::AutomotiveIndustry.serial(),
                ),
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
                (Industry::Tourism.name(), Industry::Tourism.serial()),
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
                ("貿易百貨業", 26),
                ("其他業", 33),
                ("農業科技業", 35),
            ]),
            last_revenues: RwLock::new(HashMap::new()),
            last_trading_day_quotes: RwLock::new(HashMap::new()),
            quote_history_records: RwLock::new(HashMap::new()),
        }
    }

    pub async fn load(&self) {
        let indices = index::Index::fetch().await;
        match self.indices.write() {
            Ok(mut i) => {
                if let Ok(indices) = indices {
                    i.extend(indices);
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
                    .insert(e.security_code.to_string(), e.clone());
            });
        } else {
            logging::error_file_async("Failed to update last_revenues".to_string());
        }

        let last_daily_quotes = last_daily_quotes::LastDailyQuotes::fetch().await;
        if let (Ok(result), Ok(mut ldq)) =
            (&last_daily_quotes, self.last_trading_day_quotes.write())
        {
            for e in result {
                ldq.insert(e.security_code.to_string(), e.clone());
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

        logging::info_file_async(format!(
            "CacheShare.indices 初始化 {}",
            self.indices.read().unwrap().len()
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

    /// 更新快取內股票最後的報價
    pub async fn set_stock_index(&self, key: String, index: index::Index) -> Option<index::Index> {
        match self.indices.write() {
            Ok(mut indices) => indices.insert(key, index),
            Err(_) => Some(index),
        }
    }

    /// 取得台股指數
    pub fn get_stock_index(&self, key: &str) -> Option<index::Index> {
        match self.indices.read() {
            Ok(cache) => cache.get(key).cloned(),
            Err(_) => None,
        }
    }

    /// 使用交易市場代碼取得交易市場的數據
    pub fn get_exchange_market(
        &self,
        id: i32,
    ) -> Option<stock_exchange_market::StockExchangeMarket> {
        SHARE.exchange_markets.get(&id).cloned()
    }

    /// 透過股票產業分類名稱取得對應的代碼
    pub fn get_industry_id(&self, name: &str) -> Option<i32> {
        // 如果找到了行業，則返回相應的ID。如果沒有找到，則返回99。
        match SHARE.industries.get(name) {
            None => Some(99),
            Some(industry) => Some(*industry),
        }
    }

    /// 透過股票產業分類代碼取得對應的名稱
    pub fn get_industry_name(&self, id: i32) -> Option<&'static str> {
        self.industries
            .iter()
            .find_map(|(key, &value)| if value == id { Some(key) } else { None })
            .copied()
    }

    /// 從快取中取得股票的資料
    pub async fn get_stock(&self, symbol: &str) -> Option<stock::Stock> {
        match self.stocks.read() {
            Ok(cache) => cache.get(symbol).cloned(),
            Err(_) => None,
        }
    }

    /// 從快取中取得股票最後的報價
    pub async fn get_stock_last_price(
        &self,
        symbol: &str,
    ) -> Option<last_daily_quotes::LastDailyQuotes> {
        match self.last_trading_day_quotes.read() {
            Ok(cache) => cache.get(symbol).cloned(),
            Err(_) => None,
        }
    }

    /// 更新快取內股票最後的報價
    pub async fn set_stock_last_price(&self, daily_quote: &daily_quote::DailyQuote) {
        if let Ok(mut last_trading_day_quotes) = self.last_trading_day_quotes.write() {
            if let Some(quote) = last_trading_day_quotes.get_mut(&daily_quote.security_code) {
                quote.date = daily_quote.date;
                quote.closing_price = daily_quote.closing_price;
            }
        }
    }
}

impl Default for Share {
    fn default() -> Self {
        Self::new()
    }
}

/// 時效性的快取
pub static TTL: Lazy<Ttl> = Lazy::new(Default::default);

pub struct Ttl {
    /// 每日收盤數據
    daily_quote: RwLock<ttl_cache::TtlCache<String, String>>,
    trace_quote_notify: RwLock<ttl_cache::TtlCache<String, Decimal>>,
}

//
pub trait TtlCacheInner {
    fn clear(&self);
    fn daily_quote_contains_key(&self, key: &str) -> bool;
    fn daily_quote_get(&self, key: &str) -> Option<String>;
    fn daily_quote_set(
        &self,
        key: String,
        val: String,
        duration: std::time::Duration,
    ) -> Option<String>;
    fn trace_quote_contains_key(&self, key: &str) -> bool;
    fn trace_quote_get(&self, key: &str) -> Option<Decimal>;
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
