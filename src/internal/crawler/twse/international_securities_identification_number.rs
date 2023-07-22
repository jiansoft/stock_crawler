use std::result::Result::Ok;

use anyhow::*;
use chrono::Local;
use scraper::{Html, Selector};

use crate::internal::{
    cache::SHARE,
    database::table,
    logging,
    util::{self, datetime::Weekend},
    StockExchangeMarket,
};

const REQUIRED_CATEGORIES: [&str; 4] = ["股票", "特別股", "普通股", "臺灣存託憑證(TDR)"];

/// twse 國際證券識別碼
#[derive(Debug)]
pub struct InternationalSecuritiesIdentificationNumber {
    //pub exchange: StockExchangeMarket,
    pub stock_symbol: String,
    pub name: String,
    pub isin_code: String,
    pub listing_date: String,
    //pub market_category: String,
    pub industry: String,
    pub cfi_code: String,
    pub exchange_market: table::stock_exchange_market::Entity,
    pub industry_id: i32,
}

impl Clone for InternationalSecuritiesIdentificationNumber {
    fn clone(&self) -> Self {
        InternationalSecuritiesIdentificationNumber {
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
pub async fn visit(
    mode: StockExchangeMarket,
) -> Result<Vec<InternationalSecuritiesIdentificationNumber>> {
    if Local::now().is_weekend() {
        return Ok(Vec::new());
    }

    let url = format!(
        "https://isin.twse.com.tw/isin/C_public.jsp?strMode={}",
        mode.serial_number()
    );
    logging::info_file_async(format!("visit url:{}", url,));
    let response = util::http::get_use_big5(&url).await?;
    let mut result: Vec<InternationalSecuritiesIdentificationNumber> = Vec::with_capacity(4096);
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

            let exchange_market: table::stock_exchange_market::Entity =
                match SHARE.exchange_markets.get(&mode.serial_number()) {
                    None => table::stock_exchange_market::Entity::new(
                        mode.serial_number(),
                        mode.exchange().serial_number(),
                    ),
                    Some(em) => em.clone(),
                };
            let industry_id = match SHARE.industries.get(industry.as_str()) {
                None => 99,
                Some(industry) => *industry,
            };

            let isin = InternationalSecuritiesIdentificationNumber {
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

    Ok(result)
}

#[cfg(test)]
mod tests {
    use crate::internal::cache::SHARE;
    use crate::internal::logging;

    //use std::collections::HashSet;
    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 visit".to_string());
        // let mut unique_industry_categories: HashSet<String> = HashSet::new();
        for mode in StockExchangeMarket::iterator() {
            match visit(mode).await {
                Err(why) => {
                    logging::error_file_async(format!("Failed to visit because {:?}", why));
                }
                Ok(result) => {
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
