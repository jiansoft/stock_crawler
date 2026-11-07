//! # 元大即時報價採集
//!
//! 此模組透過元大提供的即時報價 API 取得台股最新成交價與漲跌資訊。
//!
//! ## 使用端點
//!
//! - `GET /prod/yesidmz/api/basic/currentstock?symbol={symbol}`
//!
//! ## 目前狀態
//!
//! 此來源目前未納入即時股價追蹤與完整報價的站點池。
//! 原因是近期觀察到 API 回傳的是前一交易日資料，不符合即時追蹤用途。
//! 模組本身仍保留，供後續重新驗證來源品質後再決定是否恢復使用。

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{
    core::declare::StockQuotes,
    core::util::{self},
    infra::crawler::{
        StockInfo,
        yuanta::{HOST, Yuanta},
    },
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

/// 組出元大即時報價 API 的查詢網址。
///
/// 抽成純函式讓網址格式可被單元測試鎖定，避免改版時悄悄打錯端點。
fn build_url(stock_symbol: &str) -> String {
    format!(
        "https://{host}/prod/yesidmz/api/basic/currentstock?symbol={symbol}",
        host = HOST,
        symbol = stock_symbol
    )
}

/// 檢查 API 回應狀態並取出報價資料。
///
/// 這是一個純函式（不做網路 I/O），可直接用固定 JSON 樣本驗證：
/// `status` 非 `0` 時視為來源回報失敗，必須明確報錯而不是回傳空資料。
fn parse_response(url: &str, response: Response) -> Result<Data> {
    if response.status != 0 {
        return Err(anyhow!(
            "Failed to fetch_data from {url} because status is {}",
            response.status
        ));
    }

    Ok(response.data)
}

/// 向元大即時報價 API 取得指定股票代碼的原始資料。
///
/// 只負責 HTTP 抓取，狀態檢查與取值交給 [`parse_response`]。
async fn fetch_data(stock_symbol: &str) -> Result<Data> {
    let url = build_url(stock_symbol);
    let response = util::http::get_json::<Response>(&url).await?;

    parse_response(&url, response)
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
mod tests;
