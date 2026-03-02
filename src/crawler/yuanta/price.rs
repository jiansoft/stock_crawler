//! # 元大即時報價採集
//!
//! 此模組透過元大提供的即時報價 API 取得台股最新成交價與漲跌資訊。
//!
//! ## 使用端點
//!
//! - `GET /prod/yesidmz/api/basic/currentstock?symbol={symbol}`
//!
//! ## 目前用途
//!
//! - 作為 `fetch_stock_price_from_remote_site` 的備援來源
//! - 作為 `fetch_stock_quotes_from_remote_site` 的備援來源

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rust_decimal::Decimal;
use serde_derive::Deserialize;

use crate::{
    crawler::{
        yuanta::{HOST, Yuanta},
        StockInfo,
    },
    declare::StockQuotes,
    util::{self},
};

/// 元大即時報價 API 回應主體。
#[derive(Deserialize, Debug, Clone)]
struct Response {
    /// 即時報價資料。
    data: Data,
    /// API 狀態碼；`0` 表示成功。
    status: i32,
}

/// 元大即時報價資料。
#[derive(Deserialize, Debug, Clone)]
struct Data {
    /// 最新成交價。
    deal: f64,
    /// 漲跌。
    trend: f64,
    /// 漲跌幅。
    #[serde(rename = "trendPercentage")]
    trend_percentage: f64,
}

/// 向元大即時報價 API 取得指定股票代碼的原始資料。
async fn fetch_data(stock_symbol: &str) -> Result<Data> {
    let url = format!(
        "https://{host}/prod/yesidmz/api/basic/currentstock?symbol={symbol}",
        host = HOST,
        symbol = stock_symbol
    );
    let response = util::http::get_json::<Response>(&url).await?;

    if response.status != 0 {
        return Err(anyhow!(
            "Failed to fetch_data from {url} because status is {}",
            response.status
        ));
    }

    Ok(response.data)
}

#[async_trait]
impl StockInfo for Yuanta {
    /// 取得指定股票的即時成交價。
    ///
    /// # 參數
    /// * `stock_symbol` - 台股股票代碼（例如：`2330`）。
    ///
    /// # 回傳
    /// * `Result<Decimal>` - 成功時回傳最新成交價；失敗時回傳 API 或解析錯誤。
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        let data = fetch_data(stock_symbol).await?;
        Ok(Decimal::try_from(data.deal)?)
    }

    /// 取得指定股票的即時報價資訊。
    ///
    /// # 參數
    /// * `stock_symbol` - 台股股票代碼（例如：`2330`）。
    ///
    /// # 回傳
    /// * `Result<StockQuotes>` - 成功時回傳統一格式的報價資訊；
    ///   失敗時回傳 API 或解析錯誤。
    ///
    /// # 目前回填欄位
    /// * 最新成交價（`deal`）
    /// * 漲跌（`trend`）
    /// * 漲跌幅（`trendPercentage`）
    async fn get_stock_quotes(stock_symbol: &str) -> Result<StockQuotes> {
        let data = fetch_data(stock_symbol).await?;

        Ok(StockQuotes {
            stock_symbol: stock_symbol.to_string(),
            price: data.deal,
            change: data.trend,
            change_range: data.trend_percentage,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_get_stock_price() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 yuanta::get_stock_price".to_string());

        for stock_symbol in ["2330", "5306"] {
            match Yuanta::get_stock_price(stock_symbol).await {
                Ok(price) => logging::debug_file_async(format!(
                    "yuanta {stock_symbol} price: {price}"
                )),
                Err(why) => logging::debug_file_async(format!(
                    "Failed to yuanta::get_stock_price({stock_symbol}) because {:?}",
                    why
                )),
            }
        }

        logging::debug_file_async("結束 yuanta::get_stock_price".to_string());
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_stock_quotes() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 yuanta::get_stock_quotes".to_string());

        for stock_symbol in ["2330", "5306"] {
            match Yuanta::get_stock_quotes(stock_symbol).await {
                Ok(quotes) => logging::debug_file_async(format!(
                    "yuanta {stock_symbol} quotes: {:?}",
                    quotes
                )),
                Err(why) => logging::debug_file_async(format!(
                    "Failed to yuanta::get_stock_quotes({stock_symbol}) because {:?}",
                    why
                )),
            }
        }

        logging::debug_file_async("結束 yuanta::get_stock_quotes".to_string());
    }
}
