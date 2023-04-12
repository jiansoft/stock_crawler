use crate::{
    internal::cache_share::CACHE_SHARE,
    internal::database::model,
    internal::util::datetime::Weekend,
    internal::{util, StockExchangeMarket},
    logging
};
use chrono::Local;
use core::result::Result::Ok;
use scraper::{Html, Selector};

const REQUIRED_CATEGORIES: [&str; 3] = ["股票", "特別股", "普通股"];

/// twse 國際證券識別碼
#[derive(Debug)]
pub struct Entity {
    //pub exchange: StockExchangeMarket,
    pub stock_symbol: String,
    pub name: String,
    pub isin_code: String,
    pub listing_date: String,
    //pub market_category: String,
    pub industry: String,
    pub cfi_code: String,
    pub exchange_market: model::stock_exchange_market::Entity,
    pub industry_id: i32,
}

impl Clone for Entity {
    fn clone(&self) -> Self {
        Entity {
            stock_symbol: self.stock_symbol.to_string(),
            name: self.name.to_string(),
            isin_code: self.isin_code.to_string(),
            listing_date: self.listing_date.to_string(),
            industry: self.industry.to_string(),
            cfi_code: self.cfi_code.to_string(),
            exchange_market: self.exchange_market.clone(),
            industry_id: self.industry_id,
        }
    }
}

/// 調用  twse API 取得台股國際證券識別碼
/// 上市:2 上櫃︰4 興櫃︰5
pub async fn visit(mode: StockExchangeMarket) -> Option<Vec<Entity>> {
    if Local::now().is_weekend() {
        return None;
    }

    let url = format!(
        "https://isin.twse.com.tw/isin/C_public.jsp?strMode={}",
        mode.serial_number()
    );

    let response = match util::http::request_get_use_big5(&url).await {
        Ok(r) => r,
        Err(why) => {
            logging::error_file_async(format!("Failed to request_get_use_big5 because {:?}", why));
            return None;
        }
    };

    let mut result: Vec<Entity> = Vec::with_capacity(4096);
    let document = Html::parse_document(&response);

    if let Ok(selector) = Selector::parse("body > table.h4 > tbody > tr") {
        let mut is_required_category = false;
        for node in document.select(&selector).skip(1) {
            let tds: Vec<&str> = node.text().map(str::trim).collect();
            if tds.len() == 2 {
                is_required_category = REQUIRED_CATEGORIES.contains(&tds[0].trim());
                continue;
            }

            if !is_required_category {
                continue;
            }

            let split: Vec<&str> = tds[0].split('\u{3000}').collect();
            if split.len() != 2 {
                // 名稱和代碼有缺
                continue;
            }

            let industry: String;
            let cfi_code: String;

            match { tds.len() } {
                5 => {
                    industry = "未分類".to_string();
                    cfi_code = tds[4].to_owned()
                }
                6 => {
                    industry = tds[4].to_owned();
                    cfi_code = tds[5].to_owned();
                }
                _ => {
                    industry = "未分類".to_string();
                    cfi_code = "".to_string();
                }
            };

            let exchange_market: model::stock_exchange_market::Entity =
                match CACHE_SHARE.exchange_markets.get(&mode.serial_number()) {
                    None => model::stock_exchange_market::Entity::new(
                        mode.serial_number(),
                        mode.exchange().serial_number(),
                    ),
                    Some(em) => em.clone(),
                };
            let industry_id = match CACHE_SHARE.industries.get(industry.as_str()) {
                None => 0,
                Some(industry) => *industry,
            };

            let isin = Entity {
                stock_symbol: split[0].trim().to_owned(),
                name: split[1].to_owned(),
                isin_code: tds[1].to_owned(),
                listing_date: tds[2].to_owned(),
                industry,
                cfi_code,
                exchange_market,
                industry_id,
            };

            result.push(isin);
        }
    }

    Some(result)
}

#[cfg(test)]
mod tests {
    use crate::internal::cache_share::CACHE_SHARE;
    //use std::collections::HashSet;
    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        CACHE_SHARE.load().await;
        logging::debug_file_async("開始 visit".to_string());
        // let mut unique_industry_categories: HashSet<String> = HashSet::new();
        for mode in StockExchangeMarket::iterator() {
            match visit(mode).await {
                None => {
                    logging::debug_file_async(
                        "Failed to visit because response is no data".to_string(),
                    );
                }
                Some(result) => {
                    for item in result.iter() {
                        //unique_industry_categories.insert(item.industry_category.clone());
                        /* if !CACHE_SHARE.industries.contains_key(item.industry.as_str()) {
                            logging::warn_file_async(format!(
                                "stock_symbol:{} industry:{} not in industries",
                                item.stock_symbol, item.industry
                            ));
                        }*/
                        logging::debug_file_async(format!("item:{:#?}", item));
                    }
                    //logging::debug_file_async(format!("data:{:#?}", result));
                }
            }
        }

        /* for unique_industry_category in unique_industry_categories {
            logging::debug_file_async(format!("{}", unique_industry_category));
        }*/

        logging::debug_file_async("結束 visit".to_string());
    }
}
