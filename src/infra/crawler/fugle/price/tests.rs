//! Fugle 即時報價的單元測試。

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
