//! # Winvest 即時報價採集
//!
//! 此模組透過 Winvest 的 `QueryDayPrice` API 取得指定股票代碼的即時報價資訊。
//!
//! ## 使用端點
//! - `POST /Stock/Symbol/QueryDayPrice`
//! - 表單欄位：`inModel[SymbolCode]={symbol}`
//!
//! ## 資料對應
//! - `StockLastKline.ClosePrice` -> 最新成交價
//! - `StockLastKline.Change` -> 漲跌值
//! - `StockLastKline.ChangeRate` -> 漲跌幅（若為 0 且有昨收，會改用公式回推）
//!
//! ## 設計重點
//! - 先讀 `StockLastKline`，拿到最完整的報價欄位。
//! - 若缺少 `StockLastKline`，`get_stock_price` 會回退使用 `StockListPrice` 最後一筆價格。
//! - 回傳型別統一成專案內部的 `declare::StockQuotes`。

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{
    crawler::{
        winvest::{Winvest, HOST},
        StockInfo,
    },
    declare,
    util::{self, text},
};

#[derive(Deserialize, Debug, Clone)]
/// `QueryDayPrice` API 回應主體。
///
/// # 欄位說明
/// - `stock_list_price`:
///   分時資料表，通常第一列為欄位名稱（例如 `["KlineDatetime", "ClosePrice"]`）。
/// - `stock_last_kline`:
///   最新一筆 K 線摘要，包含收盤、漲跌與昨收等核心資訊。
/// - `err_msg`:
///   API 回傳的錯誤訊息；空字串或 `None` 代表未回報錯誤。
struct QueryDayPriceResponse {
    #[serde(rename = "StockListPrice", default)]
    stock_list_price: Vec<Vec<String>>,
    #[serde(rename = "StockLastKline")]
    stock_last_kline: Option<StockLastKline>,
    #[serde(rename = "errMsg", default)]
    err_msg: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
/// Winvest 最新一筆 K 線摘要。
struct StockLastKline {
    /// 最新成交（或收盤）價格。
    #[serde(rename = "ClosePrice")]
    close_price: f64,
    /// 與昨收相比的漲跌值。
    #[serde(rename = "Change")]
    change: f64,
    /// 昨日收盤價；有些情境可能為 `null`。
    #[serde(rename = "YesterdayClosePrice")]
    yesterday_close_price: Option<f64>,
    /// API 提供的漲跌幅（百分比）。
    ///
    /// 實務上可能出現 `0`，本模組會在需要時改用 `change / yesterday_close * 100` 回推。
    #[serde(rename = "ChangeRate")]
    change_rate: Option<f64>,
}

/// 呼叫 Winvest `QueryDayPrice` 並解析成結構化資料。
///
/// # 參數
/// - `stock_symbol`: 台股代碼（例如 `2330`）。
///
/// # 回傳
/// - `Ok(QueryDayPriceResponse)`：成功取得且可解析。
/// - `Err`：HTTP 失敗、JSON 解析失敗，或 API 明確回報 `errMsg`。
///
/// # 錯誤條件
/// - API 內容無法解析。
/// - API 回傳 `errMsg` 非空值。
/// - `StockLastKline` 與 `StockListPrice` 同時缺失。
async fn fetch_data(stock_symbol: &str) -> Result<QueryDayPriceResponse> {
    let url = format!("https://{host}/Stock/Symbol/QueryDayPrice", host = HOST);
    let mut params = HashMap::new();
    params.insert("inModel[SymbolCode]", stock_symbol);

    let response_text = util::http::post(&url, None, Some(params)).await?;
    let response: QueryDayPriceResponse = serde_json::from_str(&response_text).map_err(|why| {
        let preview = response_text.chars().take(300).collect::<String>();
        anyhow!(
            "Failed to parse QueryDayPrice response from {} because {:?}. body preview: {}",
            url,
            why,
            preview
        )
    })?;

    if let Some(err_msg) = response
        .err_msg
        .as_deref()
        .map(str::trim)
        .filter(|msg| !msg.is_empty())
    {
        return Err(anyhow!(
            "Failed to fetch_data from {} because errMsg is {}",
            url,
            err_msg
        ));
    }

    if response.stock_last_kline.is_none() && response.stock_list_price.is_empty() {
        return Err(anyhow!(
            "Failed to fetch_data from {} because StockLastKline and StockListPrice are empty",
            url
        ));
    }

    Ok(response)
}

/// 從 `StockListPrice` 逆向尋找最後一筆可解析的成交價。
///
/// `StockListPrice` 第一列常為標題列，因此此函式以「倒序」掃描，
/// 找到第一個可解析成 `f64` 的第二欄數值即回傳。
///
/// # 回傳
/// - `Some(price)`：找到有效價格。
/// - `None`：沒有可解析價格。
fn extract_last_price_from_stock_list(stock_list_price: &[Vec<String>]) -> Option<f64> {
    stock_list_price.iter().rev().find_map(|row| {
        row.get(1)
            .and_then(|close_price| text::parse_f64(close_price, None).ok())
    })
}

/// 計算最終漲跌幅（百分比）。
///
/// # 計算策略
/// 1. 若 API 的 `change_rate` 為有效值，且不是「有漲跌但卻為 0」的異常情況，優先採用。
/// 2. 否則若有 `yesterday_close_price`，用 `change / yesterday_close_price * 100` 回推。
/// 3. 以上皆不可用時回傳 `0.0`。
/// 4. 最終值四捨五入到小數第 2 位。
///
/// # 參數
/// - `change`: 漲跌值。
/// - `yesterday_close_price`: 昨收價。
/// - `change_rate`: API 回傳漲跌幅。
fn compute_change_range(
    change: f64,
    yesterday_close_price: Option<f64>,
    change_rate: Option<f64>,
) -> f64 {
    let raw = if let Some(rate) = change_rate {
        if rate.is_finite() && (rate != 0.0 || change == 0.0) {
            rate
        } else if let Some(yesterday_close) = yesterday_close_price {
            if yesterday_close.abs() > f64::EPSILON {
                change / yesterday_close * 100.0
            } else {
                0.0
            }
        } else {
            0.0
        }
    } else if let Some(yesterday_close) = yesterday_close_price {
        if yesterday_close.abs() > f64::EPSILON {
            change / yesterday_close * 100.0
        } else {
            0.0
        }
    } else {
        0.0
    };

    (raw * 100.0).round() / 100.0
}

#[async_trait]
impl StockInfo for Winvest {
    /// 取得指定股票的最新成交價。
    ///
    /// # 流程
    /// 1. 呼叫 `QueryDayPrice` API。
    /// 2. 優先使用 `StockLastKline.ClosePrice`。
    /// 3. 若缺少 `StockLastKline`，回退使用 `StockListPrice` 最後一筆價格。
    ///
    /// # 參數
    /// - `stock_symbol`: 股票代碼（例如 `2330`）。
    ///
    /// # 回傳
    /// - `Ok(Decimal)`: 最新成交價。
    /// - `Err`: API 或解析錯誤，或找不到可用價格。
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        let response = fetch_data(stock_symbol).await?;

        if let Some(last_kline) = response.stock_last_kline {
            return Ok(Decimal::try_from(last_kline.close_price)?);
        }

        let fallback_price = extract_last_price_from_stock_list(&response.stock_list_price)
            .ok_or_else(|| {
                anyhow!(
                    "Failed to parse latest close price from StockListPrice for {}",
                    stock_symbol
                )
            })?;
        Ok(Decimal::try_from(fallback_price)?)
    }

    /// 取得指定股票的完整報價資訊。
    ///
    /// # 內容
    /// - `price`: 最新成交價（`ClosePrice`）
    /// - `change`: 漲跌值（`Change`）
    /// - `change_range`: 漲跌幅（優先取 `ChangeRate`，必要時回推，最後四捨五入到小數第 2 位）
    ///
    /// # 參數
    /// - `stock_symbol`: 股票代碼（例如 `2330`）。
    ///
    /// # 回傳
    /// - `Ok(declare::StockQuotes)`: 統一格式報價資訊。
    /// - `Err`: API 或解析錯誤，或缺少 `StockLastKline`。
    async fn get_stock_quotes(stock_symbol: &str) -> Result<declare::StockQuotes> {
        let response = fetch_data(stock_symbol).await?;
        let last_kline = response.stock_last_kline.ok_or_else(|| {
            anyhow!(
                "Failed to parse StockLastKline from Winvest response for {}",
                stock_symbol
            )
        })?;

        Ok(declare::StockQuotes {
            stock_symbol: stock_symbol.to_string(),
            price: last_kline.close_price,
            change: last_kline.change,
            change_range: compute_change_range(
                last_kline.change,
                last_kline.yesterday_close_price,
                last_kline.change_rate,
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[test]
    /// 驗證 `StockListPrice` 會抓到最後一筆可用成交價。
    fn test_extract_last_price_from_stock_list() {
        let stock_list_price = vec![
            vec!["KlineDatetime".to_string(), "ClosePrice".to_string()],
            vec!["09:00".to_string(), "1880".to_string()],
            vec!["09:01".to_string(), "1885".to_string()],
        ];

        let price = extract_last_price_from_stock_list(&stock_list_price).unwrap();
        assert_eq!(price, 1885.0);
    }

    #[test]
    /// 驗證當 API 的 `ChangeRate` 為 0 時，會用昨收回推並四捨五入到小數第 2 位。
    fn test_compute_change_range_fallback_by_yesterday_close() {
        let change_range = compute_change_range(-20.0, Some(1900.0), Some(0.0));
        assert_eq!(change_range, -1.05);
    }

    #[tokio::test]
    /// 驗證 Winvest 可取得單一股票最新成交價。
    async fn test_get_stock_price() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 winvest::get_stock_price".to_string());

        match Winvest::get_stock_price("2330").await {
            Ok(price) => logging::debug_file_async(format!("winvest price: {}", price)),
            Err(why) => logging::debug_file_async(format!(
                "Failed to winvest::get_stock_price because {:?}",
                why
            )),
        }

        logging::debug_file_async("結束 winvest::get_stock_price".to_string());
    }

    #[tokio::test]
    /// 驗證 Winvest 可取得統一格式報價資訊。
    async fn test_get_stock_quotes() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 winvest::get_stock_quotes".to_string());

        match Winvest::get_stock_quotes("2330").await {
            Ok(quotes) => {
                dbg!(&quotes);
                logging::debug_file_async(format!("winvest quotes: {:?}", quotes))
            }
            Err(why) => logging::debug_file_async(format!(
                "Failed to winvest::get_stock_quotes because {:?}",
                why
            )),
        }

        logging::debug_file_async("結束 winvest::get_stock_quotes".to_string());
    }
}
