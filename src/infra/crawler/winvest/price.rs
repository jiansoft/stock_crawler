//! # Winvest 報價採集
//!
//! 此模組透過 Winvest 的 `QueryRecentDailyPrice` API 取得指定股票代碼的最新報價。
//!
//! ## 使用端點
//! - `POST /Stock/Symbol/QueryRecentDailyPrice`
//! - 表單欄位：`inModel[SymbolCode]={symbol}`
//! - 需帶 antiforgery cookie 與 `RequestVerificationToken` 標頭（見 [`super::session`]）
//!
//! 2026-09 改版前的 `QueryDayPrice` 已下線（一律 404）。新端點的頁面註明
//! 「盤後日 K；未取得盤中資訊授權，不提供盤中走勢」：`StockListPrice` 只剩近三個月
//! 的每日收盤（`MM/DD`），`StockLastKline` 是最近一根日 K。
//!
//! ## 資料對應
//! - `StockLastKline.ClosePrice` -> 最新成交價
//! - `StockLastKline.Change` -> 漲跌值
//! - `StockLastKline.ChangeRate` -> 漲跌幅（若為 0 且有昨收，會改用公式回推）
//!
//! ## 設計重點
//! - 只採用 `KlineDatetime` 為今天的日 K：若盤中拿到的是前一交易日的 K 線，
//!   當成最新成交價會讓追蹤判斷用到舊價格，因此直接回錯誤，讓站點池改用下一個站點。
//! - 回應不是 JSON（token 失效時為 HTTP 400 空內容）時，換一組 token 重試一次。
//! - 回傳型別統一成專案內部的 `declare::StockQuotes`。

use std::collections::HashMap;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::{Local, NaiveDate};
use reqwest::header::{COOKIE, HeaderMap, HeaderValue};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{
    core::declare,
    core::util,
    infra::crawler::{
        StockInfo,
        winvest::{HOST, Winvest, session},
    },
};

#[derive(Deserialize, Debug, Clone)]
/// `QueryRecentDailyPrice` API 回應主體。
///
/// # 欄位說明
/// - `stock_last_kline`:
///   最新一根日 K 摘要，包含收盤、漲跌與昨收等核心資訊。
/// - `err_msg`:
///   API 回傳的錯誤訊息；空字串或 `None` 代表未回報錯誤。
///
/// `StockListPrice`（近三個月每日收盤，只有 `MM/DD`）無法確認年份與是否為今天，不採用。
struct QueryRecentDailyPriceResponse {
    #[serde(rename = "StockLastKline")]
    stock_last_kline: Option<StockLastKline>,
    #[serde(rename = "errMsg", default)]
    err_msg: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
/// Winvest 最新一根日 K 摘要。
struct StockLastKline {
    /// K 線時間，例如 `2026-09-29T13:30:00`。
    #[serde(rename = "KlineDatetime")]
    kline_datetime: Option<String>,
    /// 最新成交（或收盤）價格。
    #[serde(rename = "ClosePrice")]
    close_price: f64,
    /// 與昨收相比的漲跌值。
    #[serde(rename = "Change")]
    change: f64,
    /// 昨日收盤價；有些情境可能為 `null`。
    ///
    /// 減資恢復買賣日實測會等於當日收盤（6550 2026-09-29 回 17.4），不可當成參考價。
    #[serde(rename = "YesterdayClosePrice")]
    yesterday_close_price: Option<f64>,
    /// API 提供的漲跌幅（百分比）。
    ///
    /// 實務上可能出現 `0`，本模組會在需要時改用 `change / yesterday_close * 100` 回推。
    #[serde(rename = "ChangeRate")]
    change_rate: Option<f64>,
}

impl StockLastKline {
    /// K 線所屬日期；欄位缺失或格式不符時為 `None`。
    fn date(&self) -> Option<NaiveDate> {
        let datetime = self.kline_datetime.as_deref()?;
        NaiveDate::parse_from_str(datetime.get(..10)?, "%Y-%m-%d").ok()
    }
}

/// 呼叫 Winvest `QueryRecentDailyPrice` 並解析成結構化資料。
///
/// 回應不是 JSON 時視為 token 失效，換一組工作階段重試一次。
///
/// # 錯誤條件
/// - 取得 antiforgery 工作階段失敗。
/// - 重試後仍無法解析，或 API 回傳 `errMsg` 非空值。
async fn fetch_data(stock_symbol: &str) -> Result<QueryRecentDailyPriceResponse> {
    let url = format!(
        "https://{host}/Stock/Symbol/QueryRecentDailyPrice",
        host = HOST
    );

    let current = session::current().await?;
    let mut response_text = post_query(&url, stock_symbol, &current).await?;
    if !looks_like_json(&response_text) {
        let renewed = session::renew(&current).await?;
        response_text = post_query(&url, stock_symbol, &renewed).await?;
    }

    parse_query_recent_daily_price(&url, &response_text)
}

/// 以指定工作階段送出查詢；token 放標頭，避免出現在 http 日誌的表單參數裡。
async fn post_query(url: &str, stock_symbol: &str, session: &session::Session) -> Result<String> {
    let mut headers = HeaderMap::new();
    headers.insert(COOKIE, HeaderValue::from_str(&session.cookie)?);
    headers.insert(
        "RequestVerificationToken",
        HeaderValue::from_str(&session.token)?,
    );

    let mut params = HashMap::new();
    params.insert("inModel[SymbolCode]", stock_symbol);

    util::http::post(url, Some(headers), Some(params)).await
}

/// 回應是否像 JSON 物件；token 失效時伺服器回 HTTP 400 空內容。
fn looks_like_json(body: &str) -> bool {
    body.trim_start().starts_with('{')
}

/// 解析並驗證 `QueryRecentDailyPrice` 的回應內容。
///
/// 這是一個純函式（不做網路 I/O），可直接用固定 JSON 樣本驗證：
/// 解析失敗、`errMsg` 非空，都必須明確報錯。
fn parse_query_recent_daily_price(
    url: &str,
    response_text: &str,
) -> Result<QueryRecentDailyPriceResponse> {
    let response: QueryRecentDailyPriceResponse =
        serde_json::from_str(response_text).map_err(|why| {
            let preview = response_text.chars().take(300).collect::<String>();
            anyhow!(
                "Failed to parse QueryRecentDailyPrice response from {} because {:?}. body preview: {}",
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

    Ok(response)
}

/// 取出 `today` 當天的日 K；缺少日 K 或日期不是今天都回錯誤。
fn today_kline(
    response: QueryRecentDailyPriceResponse,
    stock_symbol: &str,
    today: NaiveDate,
) -> Result<StockLastKline> {
    let kline = response.stock_last_kline.ok_or_else(|| {
        anyhow!(
            "Failed to parse StockLastKline from Winvest response for {}",
            stock_symbol
        )
    })?;

    match kline.date() {
        Some(date) if date == today => Ok(kline),
        _ => Err(anyhow!(
            "Winvest StockLastKline for {} is not today's ({}): KlineDatetime={:?}",
            stock_symbol,
            today,
            kline.kline_datetime
        )),
    }
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
    /// 取得指定股票今天的最新成交價（`StockLastKline.ClosePrice`）。
    ///
    /// # 參數
    /// - `stock_symbol`: 股票代碼（例如 `2330`）。
    ///
    /// # 回傳
    /// - `Ok(Decimal)`: 最新成交價。
    /// - `Err`: API 或解析錯誤，或最新日 K 不是今天。
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        let response = fetch_data(stock_symbol).await?;
        let kline = today_kline(response, stock_symbol, Local::now().date_naive())?;
        Ok(Decimal::try_from(kline.close_price)?)
    }

    /// 取得指定股票今天的完整報價資訊。
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
    /// - `Err`: API 或解析錯誤，或最新日 K 不是今天。
    async fn get_stock_quotes(stock_symbol: &str) -> Result<declare::StockQuotes> {
        let response = fetch_data(stock_symbol).await?;
        let kline = today_kline(response, stock_symbol, Local::now().date_naive())?;

        Ok(declare::StockQuotes {
            stock_symbol: stock_symbol.to_string(),
            price: kline.close_price,
            change: kline.change,
            change_range: compute_change_range(
                kline.change,
                kline.yesterday_close_price,
                kline.change_rate,
            ),
        })
    }
}

#[cfg(test)]
mod tests;
