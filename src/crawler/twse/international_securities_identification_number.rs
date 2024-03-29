use anyhow::Result;
use chrono::Local;
use scraper::{Html, Selector};

use crate::{
    cache::SHARE,
    crawler::twse,
    database::table,
    declare::StockExchangeMarket,
    util::{self, datetime::Weekend},
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
    pub exchange_market: table::stock_exchange_market::StockExchangeMarket,
    pub industry_id: i32,
}

impl Clone for InternationalSecuritiesIdentificationNumber {
    fn clone(&self) -> Self {
        InternationalSecuritiesIdentificationNumber {
            stock_symbol: self.stock_symbol.clone(),
            name: self.name.clone(),
            isin_code: self.isin_code.clone(),
            listing_date: self.listing_date.clone(),
            industry: self.industry.clone(),
            cfi_code: self.cfi_code.clone(),
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
        "https://isin.{}/isin/C_public.jsp?strMode={}",
        twse::HOST,
        mode.serial()
    );

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

            match tds.len() {
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

            let exchange_market: table::stock_exchange_market::StockExchangeMarket =
                match SHARE.get_exchange_market(mode.serial()) {
                    None => table::stock_exchange_market::StockExchangeMarket::new(
                        mode.serial(),
                        mode.exchange().serial_number(),
                    ),
                    Some(em) => em,
                };
            let industry_id = SHARE.get_industry_id(&industry).unwrap_or(99);
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
    use crate::{cache::SHARE, logging};

    //use std::collections::HashSet;
    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    #[tokio::test]
    #[ignore]
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
