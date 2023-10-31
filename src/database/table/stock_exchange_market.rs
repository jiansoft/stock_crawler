use anyhow::{anyhow, Result};

use crate::database;

#[derive(sqlx::Type, sqlx::FromRow, Debug)]
/// 原表名 StockExchangeMarket
pub struct StockExchangeMarket {
    pub stock_exchange_market_id: i32,
    pub stock_exchange_id: i32,
    pub code: String,
    pub name: String,
}

impl StockExchangeMarket {
    pub fn new(stock_exchange_market_id: i32, stock_exchange_id: i32) -> Self {
        StockExchangeMarket {
            stock_exchange_market_id,
            stock_exchange_id,
            code: String::from(""),
            name: String::from(""),
        }
    }

    /// 取得所有股票歷史最高、最低等數據
    pub async fn fetch() -> Result<Vec<StockExchangeMarket>> {
        sqlx::query_as::<_, StockExchangeMarket>(
            r#"
SELECT
    stock_exchange_market_id,
    stock_exchange_id,
    code,
    name
FROM
    stock_exchange_market
"#,
        )
        .fetch_all(database::get_connection())
        .await
        .map_err(|why| {
            anyhow!(
                "Failed to StockExchangeMarket::fetch from database({:#?}) because:{:?}",
                crate::config::SETTINGS.postgresql,
                why
            )
        })
    }
}

impl Clone for StockExchangeMarket {
    fn clone(&self) -> Self {
        Self {
            stock_exchange_market_id: self.stock_exchange_market_id,
            stock_exchange_id: self.stock_exchange_id,
            code: self.code.clone(),
            name: self.name.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[tokio::test]
    async fn test_fetch() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 StockExchangeMarket::fetch".to_string());
        println!("開始 StockExchangeMarket::fetch");

        match StockExchangeMarket::fetch().await {
            Ok(markets) => {
                println!("markets:{:#?}", &markets);
                logging::debug_file_async(format!("markets:{:#?}", markets));
            }
            Err(why) => {
                println!("error:{:#?}", &why);
                logging::debug_file_async(format!("{:?}", why));
            }
        }
        logging::debug_file_async("結束 StockExchangeMarket::fetch".to_string());
    }
}
