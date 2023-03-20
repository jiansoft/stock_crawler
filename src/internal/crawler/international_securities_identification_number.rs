use crate::{
    internal, internal::cache_share::CACHE_SHARE, internal::crawler::StockMarket,
    internal::request_get_big5, logging,
};
use concat_string::concat_string;
use scraper::{Html, Selector};

/// 調用  twse API 取得台股國際證券識別碼
/// 上市:2 上櫃︰4 興櫃︰5
pub async fn visit(mode: StockMarket) {
    let url = concat_string!(
        "https://isin.twse.com.tw/isin/C_public.jsp?strMode=",
        mode.serial_number().to_string()
    );

    logging::info_file_async(format!("visit url:{}", url));

    let mut new_stocks = Vec::new();
    if let Some(t) = request_get_big5(url).await {
        let document = Html::parse_document(t.as_str());
        let industries = match mode {
            StockMarket::Listed => &CACHE_SHARE.listed_market_category,
            StockMarket::OverTheCounter => &CACHE_SHARE.over_the_counter_market_category,
            StockMarket::Emerging => &CACHE_SHARE.emerging_market_category,
        };

        if let Ok(selector) = Selector::parse("body > table.h4 > tbody > tr") {
            for (tr_count, node) in document.select(&selector).enumerate() {
                if tr_count == 0 {
                    continue;
                }

                let tds: Vec<&str> = node.text().clone().enumerate().map(|(_i, v)| v).collect();

                if tds.len() != 6 {
                    continue;
                }

                if tds[0] == "有價證券代號及名稱" {
                    continue;
                }

                let split: Vec<&str> = tds[0].split('\u{3000}').collect();
                if split.len() != 2 {
                    continue;
                }

                let mut stock = internal::database::model::stock::Entity::new();
                stock.security_code = split[0].trim().to_owned();
                stock.name = split[1].to_owned();
                stock.suspend_listing = false;
                if let Some(industry) = industries.get(tds[4]) {
                    stock.category = *industry;
                }

                match CACHE_SHARE.stocks.read() {
                    Ok(stocks) => {
                        if let Some(stock_in_db) = stocks.get(&stock.security_code) {
                            if stock_in_db.category != stock.category
                                || stock_in_db.name != stock.name
                            {
                                new_stocks.push(stock);
                            }
                        } else {
                            new_stocks.push(stock);
                        }
                    }
                    Err(why) => {
                        logging::error_file_async(format!("because {:?}", why));
                    }
                }
            }
        }
    }

    for stock in new_stocks {
        match stock.upsert().await {
            Ok(_) => {
                if let Ok(mut stocks) = CACHE_SHARE.stocks.write() {
                    stocks.insert(stock.security_code.to_string(), stock.clone());
                    logging::info_file_async(format!("stock add {:?}", stock));
                }
            }
            Err(why) => {
                logging::error_file_async(format!("because {:?}", why));
            }
        }
    }
}

#[cfg(test)]
mod tests {

    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        CACHE_SHARE.load().await;
        visit(StockMarket::Listed).await;
    }

    macro_rules! aw {
        ($e:expr) => {
            tokio_test::block_on($e)
        };
    }

    #[test]
    fn test_update() {
        dotenv::dotenv().ok();
        aw!(visit(StockMarket::OverTheCounter));
    }
}
