use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rust_decimal::Decimal;
use serde_derive::{Deserialize, Serialize};

use crate::{
    crawler::{
        cnyes::{CnYes, HOST},
        StockInfo,
    },
    declare::{self, StockQuotes},
    util::{self},
};

#[derive(Serialize, Deserialize, Debug, Clone)]
struct QuotesResponse {
    #[serde(rename = "6")]
    pub current_price: f64,
    #[serde(rename = "11")]
    pub change: f64,
    #[serde(rename = "56")]
    pub change_range: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Response {
    #[serde(rename = "statusCode")]
    pub status_code: i64,
    pub message: String,
    pub data: Vec<QuotesResponse>,
}

async fn fetch_data(stock_symbol: &str) -> Result<QuotesResponse> {
    let url = format!(
        "https://ws.api.{host}/ws/api/v1/quote/quotes/TWS:{symbol}:STOCK",
        host = HOST,
        symbol = stock_symbol
    );
    let res = util::http::get_json::<Response>(&url).await?;

    if res.data.is_empty() {
        return Err(anyhow!(
            "Failed to fetch_data from {} because data is empty",
            url
        ));
    }

    Ok(res.data[0].clone())
}

#[async_trait]
impl StockInfo for CnYes {
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        let r = fetch_data(stock_symbol).await?;

        Ok(Decimal::try_from(r.current_price)?)
    }

    async fn get_stock_quotes(stock_symbol: &str) -> Result<declare::StockQuotes> {
        let r = fetch_data(stock_symbol).await?;

        Ok(StockQuotes {
            stock_symbol: stock_symbol.to_string(),
            price: r.current_price,
            change: r.change,
            change_range: r.change_range,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[tokio::test]
    async fn test_get_stock_price() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 get_stock_price".to_string());

        // match get("2330").await {
        match CnYes::get_stock_price("2330").await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("price : {:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to get_stock_price because {:?}", why));
            }
        }

        logging::debug_file_async("結束 get_stock_price".to_string());
    }

    #[tokio::test]
    async fn test_get_stock_quotes() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 get_stock_quotes".to_string());

        match CnYes::get_stock_quotes("2330").await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("get_stock_quotes : {:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to get_stock_quotes because {:?}", why));
            }
        }

        logging::debug_file_async("結束 get_stock_quotes".to_string());
    }

    #[tokio::test]
    async fn test_fetch_data() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_data".to_string());

        // match get("2330").await {
        match fetch_data("2330").await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("price : {:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to fetch_data because {:?}", why));
            }
        }

        logging::debug_file_async("結束 fetch_data".to_string());
    }
}
