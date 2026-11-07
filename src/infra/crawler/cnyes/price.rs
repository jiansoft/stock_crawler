use anyhow::{Result, anyhow};
use async_trait::async_trait;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{
    core::declare::StockQuotes,
    core::util::{self},
    infra::crawler::{
        StockInfo,
        cnyes::{CnYes, HOST},
    },
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
mod tests;
