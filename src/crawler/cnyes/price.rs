use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rust_decimal::Decimal;
use serde_derive::{Deserialize, Serialize};

use crate::{
    crawler::{
        cnyes::{CnYes, HOST},
        StockInfo,
    },
    declare::StockQuotes,
    util::{self},
};

#[derive(Serialize, Deserialize, Debug, Clone)]
struct QuotesResponse {
    #[serde(rename = "6")]
    pub current_price: Option<f64>,
    #[serde(rename = "11")]
    pub change: Option<f64>,
    #[serde(rename = "56")]
    pub change_range: Option<f64>,
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

impl QuotesResponse {
    /// 取得最新成交價；若來源以 `null` 表示暫無資料，回傳明確錯誤。
    fn required_current_price(&self, stock_symbol: &str) -> Result<f64> {
        self.current_price.ok_or_else(|| {
            anyhow!(
                "CnYes quote field `6` (current price) is null for stock {}",
                stock_symbol
            )
        })
    }

    /// 取得漲跌價差；若來源缺值則退回 0，避免完整報價反序列化直接失敗。
    fn change_or_zero(&self) -> f64 {
        self.change.unwrap_or(0.0)
    }

    /// 取得漲跌幅；若來源缺值則退回 0，避免完整報價反序列化直接失敗。
    fn change_range_or_zero(&self) -> f64 {
        self.change_range.unwrap_or(0.0)
    }
}

#[async_trait]
impl StockInfo for CnYes {
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        let r = fetch_data(stock_symbol).await?;
        let current_price = r.required_current_price(stock_symbol)?;

        Ok(Decimal::try_from(current_price)?)
    }

    async fn get_stock_quotes(stock_symbol: &str) -> Result<StockQuotes> {
        let r = fetch_data(stock_symbol).await?;
        let current_price = r.required_current_price(stock_symbol)?;

        Ok(StockQuotes {
            stock_symbol: stock_symbol.to_string(),
            price: current_price,
            change: r.change_or_zero(),
            change_range: r.change_range_or_zero(),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{crawler::log_stock_price_test, logging};

    use super::*;

    #[test]
    fn test_deserialize_response_with_null_current_price() {
        let body = r#"{
            "statusCode": 200,
            "message": "OK",
            "data": [{
                "6": null,
                "11": 0.5,
                "56": 1.2
            }]
        }"#;
        let response: Response = serde_json::from_str(body).expect("response should deserialize");
        let quote = response.data.first().expect("expected one quote row");
        let err = quote
            .required_current_price("5306")
            .expect_err("null current price should return error");

        assert!(err.to_string().contains("field `6`"));
        assert!(err.to_string().contains("5306"));
    }

    #[tokio::test]
    async fn test_get_stock_price() {
        dotenv::dotenv().ok();
        log_stock_price_test::<CnYes>("2330").await;
    }

    #[tokio::test]
    async fn test_get_stock_quotes() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 cnyes::get_stock_quotes".to_string());

        match CnYes::get_stock_quotes("2330").await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("cnyes::get_stock_quotes : {:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to cnyes::get_stock_quotes because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 cnyes::get_stock_quotes".to_string());
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
