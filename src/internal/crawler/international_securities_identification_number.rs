
use crate::{
    internal, internal::cache_share::CACHE_SHARE, internal::request_get_big5,
    logging,
};
use chrono::Local;
use concat_string::concat_string;
use scraper::{Html, Selector};


/// 台股國際證券識別碼存於資籿庫內的數據
#[derive(Default, Debug, PartialEq)]
//#[serde(rename_all = "camelCase")]
pub struct Stock {
    pub category: i32,
    pub security_code: String,
    pub name: String,
    pub create_time: chrono::DateTime<Local>,
}

/*impl Copy for Stock {

}*/
impl Clone for Stock {
    fn clone(&self) -> Self {
        Stock {
            category: self.category,
            security_code: self.security_code.clone(),
            name: self.name.clone(),
            create_time: self.create_time,
        }
    }
}

/// 市場別
pub enum StockMarket {
    /// 上市
    StockExchange,
    /// 上櫃
    OverTheCounter,
}

impl StockMarket {
    pub fn serial_number(&self) -> i32 {
        match self {
            StockMarket::StockExchange => 2,
            StockMarket::OverTheCounter => 4,
        }
    }
}
/*
impl Stock {
   pub fn new() -> Self {
       Stock {
           category: 0,
           security_code: "".to_string(),
           name: "".to_string(),
           create_time: Local::now(),
       }
   }

   pub async fn upsert(&self) -> Result<PgQueryResult, Error> {
           let sql = r#"
   insert into "Company" (
       "SecurityCode", "Name", "CategoryId", "CreateTime", "SuspendListing"
   ) values (
       $1,$2,$3,$4,false
   ) on conflict ("SecurityCode") do nothing;
           "#;
           sqlx::query(sql)
               .bind(self.security_code.as_str())
               .bind(self.name.as_str())
               .bind(self.category)
               .bind(self.create_time)
               .execute(&database::DB.pool)
               .await
       }
}
*/
/// 調用  twse API 取得台股國際證券識別碼
/// 上市:2 上櫃︰4 興櫃︰5
pub async fn visit(mode: StockMarket) {
    let url = concat_string!(
        "https://isin.twse.com.tw/isin/C_public.jsp?strMode=",
        mode.serial_number().to_string()
    );

    let mut new_stocks = Vec::new();
    if let Some(t) = request_get_big5(url).await {
        let document = Html::parse_document(t.as_str());
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

                let industries = match mode {
                    StockMarket::StockExchange => {
                        &CACHE_SHARE.listed_stock_exchange_market_category
                    }
                    StockMarket::OverTheCounter => {
                        &CACHE_SHARE.listed_over_the_counter_market_category
                    }
                };
                if let Some(industry) = industries.get(tds[4]) {
                    stock.category = *industry;
                }

                /* match mode {
                    StockMarket::StockExchange => {
                        if let Some(industry) = CACHE_SHARE
                            .listed_stock_exchange_market_category
                            .get(tds[4])
                        {
                            stock.category = *industry;
                        }
                    }
                    StockMarket::OverTheCounter => {
                        if let Some(industry) = CACHE_SHARE
                            .listed_over_the_counter_market_category
                            .get(tds[4])
                        {
                            stock.category = *industry;
                        }
                    }
                }*/

                if let Ok(stocks) = CACHE_SHARE.stocks.read() {
                    if stocks.contains_key(stock.security_code.as_str()) {
                        println!("已存在 {} {:?}", stock.security_code, stock);
                        continue;
                    }
                    new_stocks.push(stock);
                }
            }
        }
    }

    for stock in new_stocks {
        match stock.upsert().await {
            Ok(_) => {
                if let Ok(mut stocks) = CACHE_SHARE.stocks.write() {
                    stocks.insert(
                        stock.security_code.to_string(),
                        stock.clone(), //model::stock::Entity::from_isin_response(&stock),
                    );
                    logging::info_file_async(format!("stock add {:?}", stock));
                }
            }
            Err(why) => {
                logging::error_file_async(format!("because {:?}", why));
            }
        }
    }
}

/*fn get_encoding(opt: Option<String>) -> &'static Encoding {
    match opt {
        None => UTF_8,
        Some(label) => {
            match Encoding::for_label((&label).as_bytes()) {
                None => {
                    print!("{} is not a known encoding label; exiting.", label);
                    std::process::exit(-2);
                }
                Some(encoding) => encoding,
            }
        }
    }
}*/

#[cfg(test)]
mod tests {

    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        CACHE_SHARE.load().await;
        visit(StockMarket::StockExchange).await;
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
