use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use reqwest::header::{self, HeaderValue};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{
    core::config::SETTINGS,
    core::declare::StockQuotes,
    core::util,
    infra::crawler::{
        StockInfo,
        fugle::{Fugle, HOST},
    },
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

    /// 以指定時間點嘗試取得一個配額。
    ///
    /// 時間由參數傳入而非直接讀時鐘，因此這是可被單元測試完整驗證的純邏輯：
    /// 測試能自行推進「現在」來覆蓋滑動視窗、封鎖與解除封鎖等分支。
    ///
    /// # 回傳
    /// * `Ok(())` - 取得配額，呼叫端可以送出請求。
    /// * `Err` - 仍在冷卻期或視窗內已達上限，呼叫端應改用下一個備援來源。
    fn try_acquire(&mut self, now: Instant) -> Result<()> {
        // 每次進來都先清理：
        // 1. 移除視窗外（超過 60 秒）的舊請求
        // 2. 若封鎖時間已過，解除封鎖
        self.cleanup(now);

        // 若目前仍在冷卻期，直接拒絕本次 Fugle 呼叫，
        // 讓外層備援邏輯立即切到下一個網站。
        if let Some(until) = self.blocked_until {
            return Err(anyhow!(
                "Fugle local rate limit active, retry after {:?}",
                until.saturating_duration_since(now)
            ));
        }

        // 滑動視窗內的請求數已達上限時：
        // 1. 以「最早那筆請求 + 視窗長度」作為下次可恢復時間
        // 2. 進入暫時封鎖狀態，避免後續短時間內持續打到 Fugle
        // 3. 本次直接回錯，交給下一個備援來源處理
        if self.requests.len() >= LOCAL_RATE_LIMIT_PER_MINUTE {
            let next_reset = self
                .requests
                .front()
                .copied()
                .map(|oldest| oldest + RATE_LIMIT_WINDOW)
                .unwrap_or(now + RATE_LIMIT_WINDOW);
            self.blocked_until = Some(next_reset);

            return Err(anyhow!(
                "Fugle local rate limit reached ({LOCAL_RATE_LIMIT_PER_MINUTE}/min)"
            ));
        }

        // 尚未達上限時，記錄本次請求時間，
        // 代表這次 Fugle 配額已被占用。
        self.requests.push_back(now);
        Ok(())
    }

    /// 自指定時間點起強制進入一個視窗長度的冷卻期。
    fn block_from(&mut self, now: Instant) {
        self.blocked_until = Some(now + RATE_LIMIT_WINDOW);
    }
}

/// 嘗試為 Fugle 取得一個本地限流配額。
///
/// 若已達本地上限，直接回傳錯誤，讓外層備援鏈切到下一個網站。
fn acquire_rate_limit_slot() -> Result<()> {
    // 先取得全域限流器鎖，確保多執行緒下的計數與封鎖狀態一致。
    RATE_LIMITER
        .lock()
        .map_err(|_| anyhow!("Failed to lock Fugle rate limiter"))?
        .try_acquire(Instant::now())
}

/// 當上游已回報限流（例如 HTTP 429）時，強制進入冷卻期。
fn mark_remote_rate_limited() {
    if let Ok(mut limiter) = RATE_LIMITER.lock() {
        limiter.block_from(Instant::now());
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
    use super::*;

    /// 驗證官方回應的 serde 欄位對應（camelCase rename）與巢狀 lastTrade。
    #[test]
    fn quote_deserializes_official_response_shape() {
        let body = r#"{
            "symbol": "2330",
            "type": "EQUITY",
            "openPrice": 990.0,
            "closePrice": 1000.0,
            "lastPrice": 1001.0,
            "changePercent": 1.52,
            "change": 15.0,
            "lastTrade": { "price": 1002.0, "size": 5, "time": 1760000000000 }
        }"#;

        let quote: Quote = serde_json::from_str(body).unwrap();

        // 未列在結構中的欄位（symbol、type、size…）應被忽略而不是報錯。
        assert_eq!(quote.open_price, Some(990.0));
        assert_eq!(quote.change, Some(15.0));
        assert_eq!(quote.change_percent, Some(1.52));
        assert_eq!(quote.last_trade.as_ref().unwrap().price, 1002.0);
    }

    /// 視窗內未達上限時，每次請求都應取得配額並被記錄下來。
    #[test]
    fn rate_limiter_allows_requests_below_limit() {
        let mut limiter = RateLimiter::default();
        let now = Instant::now();

        for i in 0..LOCAL_RATE_LIMIT_PER_MINUTE {
            limiter
                .try_acquire(now + Duration::from_millis(i as u64))
                .unwrap_or_else(|why| panic!("第 {i} 次請求不該被限流：{why:?}"));
        }

        assert_eq!(limiter.requests.len(), LOCAL_RATE_LIMIT_PER_MINUTE);
        assert!(limiter.blocked_until.is_none(), "未超量時不該進入冷卻期");
    }

    /// 一分鐘內達到上限後，下一次請求要被擋下並進入冷卻期。
    #[test]
    fn rate_limiter_blocks_after_reaching_limit() {
        let mut limiter = RateLimiter::default();
        let start = Instant::now();

        for i in 0..LOCAL_RATE_LIMIT_PER_MINUTE {
            limiter
                .try_acquire(start + Duration::from_millis(i as u64))
                .expect("額度內不該被限流");
        }

        let err = limiter
            .try_acquire(start + Duration::from_millis(1_000))
            .expect_err("超過上限必須回錯，讓外層切換備援站點");
        assert!(err.to_string().contains("local rate limit reached"));

        // 冷卻截止時間應是「最早那筆請求 + 視窗長度」。
        assert_eq!(limiter.blocked_until, Some(start + RATE_LIMIT_WINDOW));

        // 冷卻期內再次嘗試，錯誤訊息要換成 active（附剩餘等待時間）。
        let err = limiter
            .try_acquire(start + Duration::from_secs(30))
            .expect_err("冷卻期內必須持續拒絕");
        assert!(err.to_string().contains("local rate limit active"));
    }

    /// 視窗過完後舊紀錄要被清掉，Fugle 應恢復可用。
    #[test]
    fn rate_limiter_recovers_after_window_elapsed() {
        let mut limiter = RateLimiter::default();
        let start = Instant::now();

        for i in 0..LOCAL_RATE_LIMIT_PER_MINUTE {
            limiter
                .try_acquire(start + Duration::from_millis(i as u64))
                .expect("額度內不該被限流");
        }
        limiter
            .try_acquire(start + Duration::from_millis(1_000))
            .expect_err("先讓它進入冷卻期");

        // 視窗結束後：封鎖解除、舊請求清空，重新可以取得配額。
        limiter
            .try_acquire(start + RATE_LIMIT_WINDOW + Duration::from_secs(1))
            .expect("視窗過後應恢復可用");

        assert!(limiter.blocked_until.is_none(), "過期封鎖必須被解除");
        assert_eq!(limiter.requests.len(), 1, "視窗外的舊紀錄應被清掉");
    }

    /// 上游回 429 時強制冷卻一個視窗，且冷卻結束後可自行恢復。
    #[test]
    fn rate_limiter_block_from_marks_remote_rate_limited() {
        let mut limiter = RateLimiter::default();
        let now = Instant::now();

        limiter.block_from(now);
        assert_eq!(limiter.blocked_until, Some(now + RATE_LIMIT_WINDOW));

        let err = limiter
            .try_acquire(now + Duration::from_secs(1))
            .expect_err("遠端限流期間必須跳過 Fugle");
        assert!(err.to_string().contains("local rate limit active"));

        limiter
            .try_acquire(now + RATE_LIMIT_WINDOW)
            .expect("冷卻期屆滿即可恢復");
    }

    /// 驗證即時價的取值優先序：
    /// lastTrade.price → lastPrice → closePrice → openPrice → 錯誤。
    #[test]
    fn current_price_falls_back_in_priority_order() {
        let full: Quote = serde_json::from_str(
            r#"{ "openPrice": 1.0, "closePrice": 2.0, "lastPrice": 3.0,
                 "lastTrade": { "price": 4.0 } }"#,
        )
        .unwrap();
        assert_eq!(current_price(&full).unwrap(), 4.0, "優先取 lastTrade");

        let no_trade: Quote =
            serde_json::from_str(r#"{ "openPrice": 1.0, "closePrice": 2.0, "lastPrice": 3.0 }"#)
                .unwrap();
        assert_eq!(current_price(&no_trade).unwrap(), 3.0, "次選 lastPrice");

        let close_only: Quote =
            serde_json::from_str(r#"{ "openPrice": 1.0, "closePrice": 2.0 }"#).unwrap();
        assert_eq!(current_price(&close_only).unwrap(), 2.0, "再退 closePrice");

        let open_only: Quote = serde_json::from_str(r#"{ "openPrice": 1.0 }"#).unwrap();
        assert_eq!(current_price(&open_only).unwrap(), 1.0, "最後退 openPrice");

        let empty: Quote = serde_json::from_str(r#"{}"#).unwrap();
        assert!(current_price(&empty).is_err(), "全缺值必須回錯，不能給 0");
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_stock_price() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 fugle::get_stock_price");

        for stock_symbol in ["2330", "5306"] {
            match Fugle::get_stock_price(stock_symbol).await {
                Ok(price) => {
                    tracing::debug!("fugle {stock_symbol} price: {price}")
                }
                Err(why) => tracing::debug!(
                    "Failed to fugle::get_stock_price({stock_symbol}) because {:?}",
                    why
                ),
            }
        }

        tracing::debug!("結束 fugle::get_stock_price");
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_stock_quotes() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 fugle::get_stock_quotes");

        for stock_symbol in ["2330", "5306"] {
            match Fugle::get_stock_quotes(stock_symbol).await {
                Ok(quotes) => {
                    tracing::debug!("fugle::get_stock_quotes {stock_symbol}: {:?}", quotes)
                }
                Err(why) => tracing::debug!(
                    "Failed to fugle::get_stock_quotes({stock_symbol}) because {:?}",
                    why
                ),
            }
        }

        tracing::debug!("結束 fugle::get_stock_quotes");
    }
}
