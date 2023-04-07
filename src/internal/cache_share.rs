use crate::{
    internal::{database::model::index, database::model::revenue, database::model::stock},
    logging,
    internal::database::model::last_daily_quotes
};
//use futures::executor::block_on;
use once_cell::sync::Lazy;
use std::{collections::HashMap, sync::RwLock};

pub static CACHE_SHARE: Lazy<CacheShare> = Lazy::new(Default::default);

/// CacheShare 各類快取共享集中處
pub struct CacheShare {
    /// 存放台股歷年指數
    pub indices: RwLock<HashMap<String, index::Entity>>,
    /// 存放台股股票代碼
    pub stocks: RwLock<HashMap<String, stock::Entity>>,
    /// 上市股票分類
    pub listed_market_category: HashMap<&'static str, i32>,
    /// 上櫃股票分類
    pub over_the_counter_market_category: HashMap<&'static str, i32>,
    /// 興櫃股票分類
    pub emerging_market_category: HashMap<&'static str, i32>,
    /// 月營收的快取(防止重複寫入)，第一層 Key:日期 yyyyMM 第二層 Key:股號
    pub last_revenues: RwLock<HashMap<i64, HashMap<String, revenue::Entity>>>,
    /// 存放最後交易日股票報價數據
    pub last_trading_day_quotes: RwLock<HashMap<String, last_daily_quotes::Entity>>,
}

impl CacheShare {
    pub fn new() -> Self {
        CacheShare {
            indices: RwLock::new(HashMap::new()),
            stocks: RwLock::new(HashMap::new()),
            listed_market_category: HashMap::from([
                ("水泥工業", 1),
                ("食品工業", 2),
                ("塑膠工業", 3),
                ("紡織纖維", 4),
                ("電機機械", 6),
                ("電器電纜", 7),
                ("玻璃陶瓷", 9),
                ("造紙工業", 10),
                ("鋼鐵工業", 11),
                ("橡膠工業", 12),
                ("汽車工業", 13),
                ("建材營造業", 19),
                ("航運業", 20),
                ("觀光事業", 21),
                ("金融保險業", 22),
                ("貿易百貨業", 24),
                ("存託憑證", 25),
                ("ETF", 26),
                ("受益證券", 29),
                ("其他業", 30),
                ("化學工業", 37),
                ("生技醫療業", 38),
                ("油電燃氣業", 39),
                ("半導體業", 40),
                ("電腦及週邊設備業", 41),
                ("光電業", 42),
                ("通信網路業", 43),
                ("電子零組件業", 44),
                ("電子通路業", 45),
                ("資訊服務業", 46),
                ("其他電子業", 47),
            ]),
            over_the_counter_market_category: HashMap::from([
                ("生技醫療業", 121),
                ("食品工業", 122),
                ("塑膠工業", 123),
                ("紡織纖維", 124),
                ("電機機械", 125),
                ("電器電纜", 126),
                ("鋼鐵工業", 130),
                ("橡膠工業", 131),
                ("建材營造業", 138),
                ("航運業", 139),
                ("觀光事業", 140),
                ("金融保險業", 141),
                ("貿易百貨業", 142),
                ("其他業", 145),
                ("化學工業", 151),
                ("半導體業", 153),
                ("電腦及週邊設備業", 154),
                ("光電業", 155),
                ("通信網路業", 156),
                ("電子零組件業", 157),
                ("電子通路業", 158),
                ("資訊服務業", 159),
                ("其他電子業", 160),
                ("油電燃氣業", 161),
                ("文化創意業", 169),
                ("農業科技業", 170),
                ("電子商務", 171),
                ("ETF", 172),
            ]),
            emerging_market_category: HashMap::from([
                ("生技醫療業", 1121),
                ("食品工業", 1122),
                ("塑膠工業", 1123),
                ("紡織纖維", 1124),
                ("電機機械", 1125),
                ("電器電纜", 1126),
                ("鋼鐵工業", 1130),
                ("橡膠工業", 1131),
                ("建材營造業", 1138),
                ("航運業", 1139),
                ("觀光事業", 1140),
                ("金融保險業", 1141),
                ("貿易百貨業", 1142),
                ("其他業", 1145),
                ("化學工業", 1151),
                ("半導體業", 1153),
                ("電腦及週邊設備業", 1154),
                ("光電業", 1155),
                ("通信網路業", 1156),
                ("電子零組件業", 1157),
                ("電子通路業", 1158),
                ("資訊服務業", 1159),
                ("其他電子業", 1160),
                ("油電燃氣業", 1161),
                ("文化創意業", 1169),
                ("農業科技業", 1170),
                ("電子商務", 1171),
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
                logging::error_file_async(format!("because {:?}", why));
            }
        }

        let stocks = stock::fetch().await;
        match self.stocks.write() {
            Ok(mut s) => {
                if let Ok(result) = stocks {
                    for e in result {
                        s.insert(e.stock_symbol.to_string(), e);
                    }
                }
            }
            Err(why) => {
                logging::error_file_async(format!("because {:?}", why));
            }
        }

        let revenues_from_db = revenue::fetch_last_two_month().await;
        match self.last_revenues.write() {
            Ok(mut last_revenue) => {
                if let Ok(result) = revenues_from_db {
                    for e in result {
                        last_revenue
                            .entry(e.date)
                            .or_insert_with(HashMap::new)
                            .insert(e.security_code.to_string(), e);
                    }
                }
            }
            Err(why) => {
                logging::error_file_async(format!("because {:?}", why));
            }
        }

        let last_daily_quotes_from_db = last_daily_quotes::fetch().await;
        match self.last_trading_day_quotes.write() {
            Ok(mut last_daily_quotes) => {
                if let Ok(result) = last_daily_quotes_from_db {
                    for e in result {
                        last_daily_quotes
                            .entry(e.security_code.to_string())
                            .or_insert(e.clone());
                    }
                }
            }
            Err(why) => {
                logging::error_file_async(format!("because {:?}", why));
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

            for (k, v) in CACHE_SHARE.listed_market_category.iter() {
                logging::info_file_async(format!("name {}  category {}", k, v));
            }
        });
    }
}
