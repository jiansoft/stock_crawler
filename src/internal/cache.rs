use std::sync::RwLock;

use hashbrown::HashMap;
use once_cell::sync::Lazy;

//use futures::executor::block_on;
use crate::internal::{
    database::table::{
        index, last_daily_quotes, quote_history_record, revenue, stock, stock_exchange_market,
    },
    logging,
};

pub static SHARE: Lazy<Share> = Lazy::new(Default::default);

/// Share 各類快取共享集中處
pub struct Share {
    /// 存放台股歷年指數
    pub indices: RwLock<HashMap<String, index::Index>>,
    /// 存放台股股票代碼
    pub stocks: RwLock<HashMap<String, stock::Stock>>,
    /// 月營收的快取(防止重複寫入)，第一層 Key:日期 yyyyMM 第二層 Key:股號
    pub last_revenues: RwLock<HashMap<i64, HashMap<String, revenue::Revenue>>>,
    /// 存放最後交易日股票報價數據
    pub last_trading_day_quotes: RwLock<HashMap<String, last_daily_quotes::Entity>>,
    // quote_history_records 股票歷史、淨值比等最高、最低的數據,resource.Init() 從資料庫內讀取出，若抓到新的數據時則會同時更新資料庫與此數據
    pub quote_history_records: RwLock<HashMap<String, quote_history_record::QuoteHistoryRecord>>,

    /// 股票產業分類
    pub industries: HashMap<&'static str, i32>,
    /// 股票產業分類(2, 'TAI', '上市', 1),(4, 'TWO', '上櫃', 2), (5, 'TWE', '興櫃', 2);
    pub exchange_markets: HashMap<i32, stock_exchange_market::Entity>,
}

impl Share {
    pub fn new() -> Self {
        Share {
            indices: RwLock::new(HashMap::new()),
            stocks: RwLock::new(HashMap::new()),
            exchange_markets: HashMap::from([
                (
                    2,
                    stock_exchange_market::Entity {
                        stock_exchange_market_id: 2,
                        stock_exchange_id: 1,
                        code: "TAI".to_string(),
                        name: "上市".to_string(),
                    },
                ),
                (
                    4,
                    stock_exchange_market::Entity {
                        stock_exchange_market_id: 4,
                        stock_exchange_id: 2,
                        code: "TWO".to_string(),
                        name: "上櫃".to_string(),
                    },
                ),
                (
                    5,
                    stock_exchange_market::Entity {
                        stock_exchange_market_id: 5,
                        stock_exchange_id: 2,
                        code: "TWE".to_string(),
                        name: "興櫃".to_string(),
                    },
                ),
            ]),
            industries: HashMap::from([
                ("水泥工業", 1),
                ("食品工業", 2),
                ("塑膠工業", 3),
                ("紡織纖維", 4),
                ("電機機械", 5),
                ("電器電纜", 6),
                ("化學工業", 7),
                ("生技醫療業", 8),
                ("玻璃陶瓷", 9),
                ("造紙工業", 10),
                ("鋼鐵工業", 11),
                ("橡膠工業", 12),
                ("汽車工業", 13),
                ("半導體業", 14),
                ("電腦及週邊設備業", 15),
                ("光電業", 16),
                ("通信網路業", 17),
                ("電子零組件業", 18),
                ("電子通路業", 19),
                ("資訊服務業", 20),
                ("其他電子業", 21),
                ("建材營造業", 22),
                ("航運業", 23),
                ("觀光事業", 24),
                ("金融保險業", 25),
                ("貿易百貨", 26),
                ("貿易百貨業", 26),
                ("油電燃氣業", 27),
                ("綜合", 28),
                ("綠能環保", 29),
                ("數位雲端", 30),
                ("運動休閒", 31),
                ("居家生活", 32),
                ("其他", 33),
                ("其他業", 33),
                ("文化創意業", 34),
                ("農業科技", 35),
                ("農業科技業", 35),
                ("電子商務", 36),
                ("觀光餐旅", 37),
                ("存託憑證", 38),
                ("未分類", 99),
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

        let last_daily_quotes = last_daily_quotes::Entity::fetch().await;
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
}

//
pub trait TtlCacheInner {
    fn daily_quote_contains_key(&self, key: &str) -> bool;
    fn daily_quote_get(&self, key: &str) -> Option<String>;
    fn daily_quote_set(
        &self,
        key: String,
        val: String,
        duration: std::time::Duration,
    ) -> Option<String>;
}

impl TtlCacheInner for Ttl {
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

    fn daily_quote_set(
        &self,
        key: String,
        val: String,
        duration: std::time::Duration,
    ) -> Option<String> {
        match self.daily_quote.write() {
            Ok(mut ttl) => ttl.insert(key, val, duration),
            Err(_) => None,
        }
    }
}

impl Ttl {
    pub fn new() -> Self {
        Ttl {
            daily_quote: RwLock::new(ttl_cache::TtlCache::new(2048)),
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
    use rust_decimal::Decimal;
    use std::thread;
    use std::time::Duration;

    use super::*;

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
        thread::sleep(Duration::from_secs(1));

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
                },
               Err(_) => todo!()
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
