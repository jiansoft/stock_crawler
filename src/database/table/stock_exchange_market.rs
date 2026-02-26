use anyhow::{anyhow, Result};

use crate::database;

/// 原表名 StockExchangeMarket
#[derive(sqlx::Type, sqlx::FromRow, Debug)]
pub struct StockExchangeMarket {
    /// 市場識別碼。
    pub stock_exchange_market_id: i32,
    /// 交易所識別碼。
    pub stock_exchange_id: i32,
    /// 市場代碼。
    pub code: String,
    /// 市場名稱。
    pub name: String,
}

impl StockExchangeMarket {
    /// 建立市場資料預設值（`code/name` 會先給空字串）。
    pub fn new(stock_exchange_market_id: i32, stock_exchange_id: i32) -> Self {
        StockExchangeMarket {
            stock_exchange_market_id,
            stock_exchange_id,
            code: String::from(""),
            name: String::from(""),
        }
    }

    /// 取得所有市場對照資料。
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
