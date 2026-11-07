//! # BigGo 即時報價
//!
//! 單檔 `/stock/current/price`，供即時報價備援池（[`StockInfo`]）使用。
//!
//! ## 回應規則（2026-10-05 盤中實測）
//!
//! - `current_price`、`daily_change`、`daily_change_percent` 為 JSON 數字；`snapshot_date`
//!   是這筆報價的時間（epoch 秒）。
//! - **當天還沒成交的股票，BigGo 回的是最後一次成交那天的快照**：1423 在 10-05 盤中回的是
//!   10-02 的報價，連昨收也是再前一天的。為了不把舊資料當成即時價，`snapshot_date` 不是
//!   台北時間今天就回錯誤，讓站點池改問下一個站點。

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use rust_decimal::{Decimal, prelude::FromPrimitive};
use serde::Deserialize;

use super::{BigGo, api_symbol, fetch};
use crate::{core::declare::StockQuotes, infra::crawler::StockInfo};

/// 台北時區（UTC+8）。
const TAIPEI_OFFSET_SECONDS: i32 = 8 * 60 * 60;

/// `/stock/current/price` 的回應資料（只取需要的欄位）。
#[derive(Debug, Deserialize)]
struct CurrentPrice {
    /// 最新成交價。
    current_price: Option<f64>,
    /// 漲跌（元）。
    daily_change: Option<f64>,
    /// 漲跌幅（%）。
    daily_change_percent: Option<f64>,
    /// 報價時間（epoch 秒）。
    snapshot_date: Option<i64>,
}

/// 驗證過的即時報價。
#[derive(Debug, Clone, Copy, PartialEq)]
struct Quote {
    /// 最新成交價。
    price: f64,
    /// 漲跌（元）。
    change: f64,
    /// 漲跌幅（%）。
    change_range: f64,
}

#[async_trait]
impl StockInfo for BigGo {
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        let quote = fetch_quote(stock_symbol).await?;
        Decimal::from_f64(quote.price)
            .map(|price| price.round_dp(4).normalize())
            .ok_or_else(|| {
                anyhow!(
                    "BigGo price {} for {stock_symbol} is out of range",
                    quote.price
                )
            })
    }

    async fn get_stock_quotes(stock_symbol: &str) -> Result<StockQuotes> {
        let quote = fetch_quote(stock_symbol).await?;
        Ok(StockQuotes {
            stock_symbol: stock_symbol.to_string(),
            price: quote.price,
            change: quote.change,
            change_range: quote.change_range,
        })
    }
}

/// 抓取單檔即時報價並驗證是今天的資料。
async fn fetch_quote(stock_symbol: &str) -> Result<Quote> {
    let symbol = api_symbol(stock_symbol);
    let data: Option<CurrentPrice> =
        fetch("/stock/current/price", &[("stock_id", &symbol)]).await?;
    let data = data.ok_or_else(|| anyhow!("BigGo has no current price for {stock_symbol}"))?;
    validate(stock_symbol, data, taipei_today())
}

/// 檢查報價完整且是 `today`（台北時間）的資料。
fn validate(stock_symbol: &str, data: CurrentPrice, today: NaiveDate) -> Result<Quote> {
    let price = data
        .current_price
        .filter(|price| *price > 0.0)
        .ok_or_else(|| anyhow!("BigGo current price for {stock_symbol} is missing"))?;
    let snapshot = data
        .snapshot_date
        .and_then(taipei_date)
        .ok_or_else(|| anyhow!("BigGo snapshot time for {stock_symbol} is missing"))?;
    if snapshot != today {
        bail!("BigGo quote for {stock_symbol} is from {snapshot}, not today ({today})");
    }
    let change = data
        .daily_change
        .ok_or_else(|| anyhow!("BigGo daily change for {stock_symbol} is missing"))?;
    let change_range = data
        .daily_change_percent
        .ok_or_else(|| anyhow!("BigGo daily change percent for {stock_symbol} is missing"))?;
    Ok(Quote {
        price,
        change,
        change_range,
    })
}

/// epoch 秒換算成台北時間的日期。
fn taipei_date(epoch_seconds: i64) -> Option<NaiveDate> {
    let offset = FixedOffset::east_opt(TAIPEI_OFFSET_SECONDS)?;
    DateTime::from_timestamp(epoch_seconds, 0).map(|time| time.with_timezone(&offset).date_naive())
}

/// 台北時間的今天（與主機時區無關）。
fn taipei_today() -> NaiveDate {
    let offset = FixedOffset::east_opt(TAIPEI_OFFSET_SECONDS).expect("UTC+8 is a valid offset");
    Utc::now().with_timezone(&offset).date_naive()
}

#[cfg(test)]
mod tests;
