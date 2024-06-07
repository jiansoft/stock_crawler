use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rust_decimal::Decimal;
use serde_derive::{Deserialize, Serialize};

use crate::{
    crawler::{
        nstock::{NStock, HOST},
        StockInfo,
    },
    declare::StockQuotes,
    util::{self, text},
};

#[derive(Serialize, Deserialize, Debug, Clone)]
struct RealTimeQuotesResponse {
    #[serde(rename = "股票代號")]
    pub stock_symbol: String,
    #[serde(rename = "股票名稱")]
    pub name: String,
    #[serde(rename = "開盤價")]
    pub opening_price: String,
    #[serde(rename = "最高價")]
    pub highest_price: String,
    #[serde(rename = "最低價")]
    pub lowest_price: String,
    #[serde(rename = "當盤成交價")]
    pub current_price: String,
    #[serde(rename = "漲跌")]
    pub change: String,
    #[serde(rename = "漲跌幅")]
    pub change_range: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Response {
    pub data: Vec<RealTimeQuotesResponse>,
}

async fn fetch_data(stock_symbol: &str) -> Result<RealTimeQuotesResponse> {
    let url = format!(
        "https://{host}/api/v2/real-time-quotes/data?stock_id={symbol}",
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
impl StockInfo for NStock {
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        let r = fetch_data(stock_symbol).await?;
        text::parse_decimal(&r.current_price, None)
    }

    async fn get_stock_quotes(stock_symbol: &str) -> Result<StockQuotes> {
        let r = fetch_data(stock_symbol).await?;

        let price = text::parse_f64(&r.current_price, None)?;
        let change = text::parse_f64(&r.change, None)?;
        let change_range = text::parse_f64(&r.change_range, None)?;
        Ok(StockQuotes {
            stock_symbol: stock_symbol.to_string(),
            price,
            change,
            change_range,
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
        match NStock::get_stock_price("2330").await {
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

        match NStock::get_stock_quotes("2330").await {
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
}
