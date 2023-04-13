//use futures::executor::block_on;
use crate::{
    internal::{
        database::model, database::model::index, database::model::last_daily_quotes,
        database::model::revenue, database::model::stock,
    },
    logging,
};
use once_cell::sync::Lazy;
use std::{collections::HashMap, sync::RwLock};

pub static CACHE_SHARE: Lazy<CacheShare> = Lazy::new(Default::default);

/// CacheShare 各類快取共享集中處
pub struct CacheShare {
    /// 存放台股歷年指數
    pub indices: RwLock<HashMap<String, index::Entity>>,
    /// 存放台股股票代碼
    pub stocks: RwLock<HashMap<String, stock::Entity>>,
    /// 月營收的快取(防止重複寫入)，第一層 Key:日期 yyyyMM 第二層 Key:股號
    pub last_revenues: RwLock<HashMap<i64, HashMap<String, revenue::Entity>>>,
    /// 存放最後交易日股票報價數據
    pub last_trading_day_quotes: RwLock<HashMap<String, last_daily_quotes::Entity>>,
    /// 股票產業分類
    pub industries: HashMap<&'static str, i32>,
    /// 股票產業分類(2, 'TAI', '上市', 1),(4, 'TWO', '上櫃', 2), (5, 'TWE', '興櫃', 2);
    pub exchange_markets: HashMap<i32, model::stock_exchange_market::Entity>,
}

impl CacheShare {
    pub fn new() -> Self {
        CacheShare {
            indices: RwLock::new(HashMap::new()),
            stocks: RwLock::new(HashMap::new()),
            exchange_markets: HashMap::from([
                (
                    2,
                    model::stock_exchange_market::Entity {
                        stock_exchange_market_id: 2,
                        stock_exchange_id: 1,
                        code: "TAI".to_string(),
                        name: "上市".to_string(),
                    },
                ),
                (
                    4,
                    model::stock_exchange_market::Entity {
                        stock_exchange_market_id: 4,
                        stock_exchange_id: 2,
                        code: "TWO".to_string(),
                        name: "上櫃".to_string(),
                    },
                ),
                (
                    5,
                    model::stock_exchange_market::Entity {
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
        }
    }

    pub async fn load(&self) -> Option<()> {
        let indices = index::fetch().await;
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

        let stocks = stock::Entity::fetch().await;
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

        let last_daily_quotes = last_daily_quotes::fetch().await;
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

        if let Ok(revenues) = self.last_revenues.read() {
            for revenue in revenues.iter() {
                logging::info_file_async(format!(
                    "CacheShare.last_revenues 初始化 {}:{}",
                    revenue.0,
                    revenue.1.keys().len()
                ));
            }
        }

        Some(())
    }
}

impl Default for CacheShare {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{thread, time};

    #[tokio::test]
    async fn test_init() {
        dotenv::dotenv().ok();
        let _ = CACHE_SHARE.indices.read().is_ok();
        thread::sleep(time::Duration::from_secs(1));
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
            CACHE_SHARE.load().await;
            let mut loop_count = 10;
            for e in CACHE_SHARE.indices.read().unwrap().iter() {
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
            for (k, v) in CACHE_SHARE.stocks.read().unwrap().iter() {
                if loop_count < 0 {
                    break;
                }

                logging::info_file_async(format!("stock {} name {}", k, v.name));
                loop_count -= 1;
            }

            loop_count = 10;
            for (k, v) in CACHE_SHARE.last_trading_day_quotes.read().unwrap().iter() {
                if loop_count < 0 {
                    break;
                }

                logging::info_file_async(format!(
                    "security_code {} closing_price {}",
                    k, v.closing_price
                ));
                loop_count -= 1;
            }

            for (k, v) in CACHE_SHARE.industries.iter() {
                logging::info_file_async(format!("name {}  category {}", k, v));
            }
        });
    }
}
