use crate::{
    internal::bot,
    internal::cache_share::CACHE_SHARE,
    internal::crawler::StockMarket,
    internal::database::model::stock,
    internal::util,
    logging,
    internal::util::datetime::Weekend
};
use anyhow::*;
use chrono::{Local};
use concat_string::concat_string;
use core::result::Result::Ok;
use scraper::{Html, Selector};

const REQUIRED_CATEGORIES: [&str; 3] = ["股票", "特別股", "普通股"];

/// 調用  twse API 取得台股國際證券識別碼
/// 上市:2 上櫃︰4 興櫃︰5
pub async fn visit() -> Result<()> {
    if  Local::now().is_weekend() {
        return Ok(());
    }

    let mut new_stocks = Vec::new();

    for mode in StockMarket::iterator() {
        let url = concat_string!(
            "https://isin.twse.com.tw/isin/C_public.jsp?strMode=",
            mode.serial_number().to_string()
        );

        logging::info_file_async(format!("visit url:{}", url));

        if let Ok(response) = util::http::request_get_use_big5(&url).await {
            let document = Html::parse_document(&response);
            let industries = match mode {
                StockMarket::Listed => &CACHE_SHARE.listed_market_category,
                StockMarket::OverTheCounter => &CACHE_SHARE.over_the_counter_market_category,
                StockMarket::Emerging => &CACHE_SHARE.emerging_market_category,
            };

            let mut is_required_category = false;

            if let Ok(selector) = Selector::parse("body > table.h4 > tbody > tr") {
                let new_stock_from_page: Vec<stock::Entity> = document
                    .select(&selector)
                    .skip(1)
                    .filter_map(|node| {
                        let tds: Vec<&str> = node.text().map(str::trim).collect();
                        if tds.len() == 2 {
                            is_required_category = REQUIRED_CATEGORIES.contains(&tds[0].trim());
                            return None;
                        }

                        if !is_required_category {
                            return None;
                        }

                        let split: Vec<&str> = tds[0].split('\u{3000}').collect();
                        if split.len() != 2 {
                            // 名稱和代碼有缺
                            return None;
                        }
                        logging::debug_file_async(format!("tds{:?}", tds));
                        let mut stock = stock::Entity::new();
                        stock.stock_symbol = split[0].trim().to_owned();
                        stock.name = split[1].to_owned();
                        stock.suspend_listing = false;
                        if let Some(industry) = industries.get(tds[4]) {
                            stock.category = *industry;
                        }
                        match CACHE_SHARE.stocks.read() {
                            Ok(stocks_cache) => {
                                return match stocks_cache.get(&stock.stock_symbol) {
                                    Some(stock_in_db)
                                        if stock_in_db.category != stock.category
                                            || stock_in_db.name != stock.name =>
                                    {
                                        Some(stock)
                                    }
                                    None => Some(stock),
                                    _ => None,
                                };
                            }
                            Err(why) => {
                                logging::error_file_async(format!(
                                    "Failed to stocks.read because {:?}",
                                    why
                                ));
                            }
                        }
                        None
                    })
                    .collect();

                new_stocks.extend(new_stock_from_page);
            }
        }
    }

    for stock in new_stocks {
        match stock.upsert().await {
            Ok(_) => {
                let msg = format!("stock add {:?}", stock);
                if let Ok(mut stocks) = CACHE_SHARE.stocks.write() {
                    stocks.insert(stock.stock_symbol.to_string(), stock.clone());
                }

                //todo 需要通知另一個服務已新增加一個股票代號
                if let Err(why) = bot::telegram::send_to_allowed(&msg).await {
                    logging::error_file_async(format!(
                        "Failed to send_to_allowed because {:?}",
                        why
                    ));
                }

                logging::info_file_async(msg);
            }
            Err(why) => {
                logging::error_file_async(format!("Failed to stock.upsert because {:?}", why));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        CACHE_SHARE.load().await;

        match visit().await {
            Ok(_) => {
                logging::info_file_async("visit executed successfully.".to_string());
            }
            Err(why) => {
                logging::error_file_async(format!("Failed to visit because {:?}", why));
            }
        };
    }
}
