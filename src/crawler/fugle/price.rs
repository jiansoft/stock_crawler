use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Duration, Instant},
};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use reqwest::header::{self, HeaderValue};
use rust_decimal::Decimal;
use serde_derive::Deserialize;

use crate::{
    crawler::{
        fugle::{Fugle, HOST},
        StockInfo,
    },
    config::SETTINGS,
    declare::StockQuotes,
    util,
};

/// Fugle 官方限制為 60 次 / 分鐘，這裡保留安全餘量避免撞線。
const LOCAL_RATE_LIMIT_PER_MINUTE: usize = 60;
/// 限流統計視窗長度。
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);

/// Fugle 本地限流狀態。
///
/// 使用滑動視窗記錄最近成功送出的請求時間，
/// 並在達到上限時暫時跳過 Fugle，交由下一個報價來源接手。
static RATE_LIMITER: Lazy<Mutex<RateLimiter>> = Lazy::new(|| Mutex::new(RateLimiter::default()));

/// Fugle 日內即時報價回應。
///
/// 對應官方 `GET /intraday/quote/{symbol}` 回傳格式，
/// 僅保留目前抓價與報價所需欄位。
#[derive(Deserialize, Debug, Clone)]
struct Quote {
    /// 開盤價。
    #[serde(rename = "openPrice")]
    open_price: Option<f64>,
    /// 收盤價（最後成交價）。
    #[serde(rename = "closePrice")]
    close_price: Option<f64>,
    /// 最後一筆成交價（含試撮）。
    #[serde(rename = "lastPrice")]
    last_price: Option<f64>,
    /// 最後一筆成交漲跌幅（含試撮）。
    #[serde(rename = "changePercent")]
    change_percent: Option<f64>,
    /// 最後一筆成交漲跌（含試撮）。
    change: Option<f64>,
    /// 最後一筆成交明細。
    #[serde(rename = "lastTrade")]
    last_trade: Option<Trade>,
}

/// Fugle 最後一筆成交資訊。
#[derive(Deserialize, Debug, Clone)]
struct Trade {
    /// 最後一筆成交價格。
    price: f64,
}

/// Fugle 本地限流器狀態。
///
/// 以滑動視窗方式記錄最近一分鐘內已送出的請求，
/// 並在達到本地限制或上游回報 429 時暫時封鎖 Fugle。
#[derive(Default)]
struct RateLimiter {
    /// 最近一個統計視窗內已送出的請求時間點。
    requests: VecDeque<Instant>,
    /// 封鎖截止時間；在此時間之前會直接跳過 Fugle。
    blocked_until: Option<Instant>,
}

impl RateLimiter {
    /// 清掉視窗外的舊請求紀錄與過期封鎖。
    fn cleanup(&mut self, now: Instant) {
        while let Some(oldest) = self.requests.front() {
            if now.duration_since(*oldest) >= RATE_LIMIT_WINDOW {
                self.requests.pop_front();
            } else {
                break;
            }
        }

        if self.blocked_until.is_some_and(|until| now >= until) {
            self.blocked_until = None;
        }
    }
}

/// 嘗試為 Fugle 取得一個本地限流配額。
///
/// 若已達本地上限，直接回傳錯誤，讓外層備援鏈切到下一個網站。
fn acquire_rate_limit_slot() -> Result<()> {
    let now = Instant::now();
    // 先取得全域限流器鎖，確保多執行緒下的計數與封鎖狀態一致。
    let mut limiter = RATE_LIMITER
        .lock()
        .map_err(|_| anyhow!("Failed to lock Fugle rate limiter"))?;

    // 每次進來都先清理：
    // 1. 移除視窗外（超過 60 秒）的舊請求
    // 2. 若封鎖時間已過，解除封鎖
    limiter.cleanup(now);

    // 若目前仍在冷卻期，直接拒絕本次 Fugle 呼叫，
    // 讓外層備援邏輯立即切到下一個網站。
    if let Some(until) = limiter.blocked_until {
        return Err(anyhow!(
            "Fugle local rate limit active, retry after {:?}",
            until.saturating_duration_since(now)
        ));
    }

    // 滑動視窗內的請求數已達上限時：
    // 1. 以「最早那筆請求 + 視窗長度」作為下次可恢復時間
    // 2. 進入暫時封鎖狀態，避免後續短時間內持續打到 Fugle
    // 3. 本次直接回錯，交給下一個備援來源處理
    if limiter.requests.len() >= LOCAL_RATE_LIMIT_PER_MINUTE {
        let next_reset = limiter
            .requests
            .front()
            .copied()
            .map(|oldest| oldest + RATE_LIMIT_WINDOW)
            .unwrap_or(now + RATE_LIMIT_WINDOW);
        limiter.blocked_until = Some(next_reset);

        return Err(anyhow!(
            "Fugle local rate limit reached ({LOCAL_RATE_LIMIT_PER_MINUTE}/min)"
        ));
    }

    // 尚未達上限時，記錄本次請求時間，
    // 代表這次 Fugle 配額已被占用。
    limiter.requests.push_back(now);
    Ok(())
}

/// 當上游已回報限流（例如 HTTP 429）時，強制進入冷卻期。
fn mark_remote_rate_limited() {
    if let Ok(mut limiter) = RATE_LIMITER.lock() {
        limiter.blocked_until = Some(Instant::now() + RATE_LIMIT_WINDOW);
    }
}

/// 建立 Fugle API 請求標頭。
///
/// 目前會從 `SETTINGS.fugle.api_key` 讀取 API Key，
/// 並填入 `X-API-KEY` 標頭。
fn build_headers() -> Result<header::HeaderMap> {
    let api_key = SETTINGS.fugle.api_key.trim();
    if api_key.is_empty() {
        return Err(anyhow!("FUGLE_API_KEY is not set"));
    }
    let mut headers = header::HeaderMap::new();
    headers.insert("X-API-KEY", HeaderValue::from_str(api_key)?);
    Ok(headers)
}

/// 向 Fugle 取得指定股票代碼的日內即時報價原始資料。
async fn fetch_data(stock_symbol: &str) -> Result<Quote> {
    acquire_rate_limit_slot()?;

    let url = format!(
        "https://{host}/marketdata/v1.0/stock/intraday/quote/{symbol}",
        host = HOST,
        symbol = stock_symbol
    );
    let res = util::http::get_response(&url, Some(build_headers()?)).await?;

    if res.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        mark_remote_rate_limited();
        return Err(anyhow!(
            "Fugle remote rate limit reached (HTTP 429), skip to fallback site"
        ));
    }

    res.json::<Quote>().await.map_err(Into::into)
}

/// 從 Fugle 回應中挑選目前最適合作為即時價的欄位。
///
/// 優先順序：
/// 1. `lastTrade.price`
/// 2. `lastPrice`
/// 3. `closePrice`
/// 4. `openPrice`
fn current_price(quote: &Quote) -> Result<f64> {
    if let Some(trade) = quote.last_trade.as_ref() {
        return Ok(trade.price);
    }
    if let Some(last_price) = quote.last_price {
        return Ok(last_price);
    }
    if let Some(close_price) = quote.close_price {
        return Ok(close_price);
    }
    if let Some(open_price) = quote.open_price {
        return Ok(open_price);
    }
    Err(anyhow!("Fugle quote price is empty"))
}

#[async_trait]
impl StockInfo for Fugle {
    /// 取得指定股票的即時成交價。
    ///
    /// # 參數
    /// * `stock_symbol` - 台股股票代碼（例如：`2330`）。
    ///
    /// # 回傳
    /// * `Result<Decimal>` - 成功時回傳最新成交價；
    ///   失敗時回傳 API 金鑰缺失、限流、HTTP 或解析錯誤。
    ///
    /// # 說明
    /// * 呼叫前會先套用 Fugle 本地限流保護。
    /// * 若 Fugle 因本地限制或上游 429 失敗，外層備援機制可切到下一個站點。
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        let quote = fetch_data(stock_symbol).await?;
        Ok(Decimal::try_from(current_price(&quote)?)?)
    }

    /// 取得指定股票的即時報價資訊。
    ///
    /// # 參數
    /// * `stock_symbol` - 台股股票代碼（例如：`2330`）。
    ///
    /// # 回傳
    /// * `Result<StockQuotes>` - 成功時回傳統一格式的即時報價；
    ///   失敗時回傳 API 金鑰缺失、限流、HTTP 或解析錯誤。
    ///
    /// # 目前回填欄位
    /// * 最新價格
    /// * 漲跌
    /// * 漲跌幅
    ///
    /// # 說明
    /// * 呼叫前會先套用 Fugle 本地限流保護。
    /// * 若 Fugle 因本地限制或上游 429 失敗，外層備援機制可切到下一個站點。
    async fn get_stock_quotes(stock_symbol: &str) -> Result<StockQuotes> {
        let quote = fetch_data(stock_symbol).await?;

        Ok(StockQuotes {
            stock_symbol: stock_symbol.to_string(),
            price: current_price(&quote)?,
            change: quote.change.unwrap_or_default(),
            change_range: quote.change_percent.unwrap_or_default(),
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
        logging::debug_file_async("開始 fugle::get_stock_price".to_string());

        for stock_symbol in ["2330", "5306"] {
            match Fugle::get_stock_price(stock_symbol).await {
                Ok(price) => logging::debug_file_async(format!(
                    "fugle {stock_symbol} price: {price}"
                )),
                Err(why) => logging::debug_file_async(format!(
                    "Failed to fugle::get_stock_price({stock_symbol}) because {:?}",
                    why
                )),
            }
        }

        logging::debug_file_async("結束 fugle::get_stock_price".to_string());
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_stock_quotes() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fugle::get_stock_quotes".to_string());

        for stock_symbol in ["2330", "5306"] {
            match Fugle::get_stock_quotes(stock_symbol).await {
                Ok(quotes) => logging::debug_file_async(format!(
                    "fugle {stock_symbol} quotes: {:?}",
                    quotes
                )),
                Err(why) => logging::debug_file_async(format!(
                    "Failed to fugle::get_stock_quotes({stock_symbol}) because {:?}",
                    why
                )),
            }
        }

        logging::debug_file_async("結束 fugle::get_stock_quotes".to_string());
    }
}
