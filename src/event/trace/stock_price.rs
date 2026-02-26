//! # 股票價格追蹤與提醒模組
//!
//! 此模組負責監控使用者設定的追蹤股票（Trace），並在股價超過預設的高低標時發送通知。
//!
//! ## 主要流程
//! 1. **檢查開盤狀態**：判斷當前是否為交易日（非週末且非假日）。
//! 2. **定期掃描**：在開盤期間，每分鐘執行一次追蹤檢查。
//! 3. **獲取即時報價**：從遠端來源（如 Yahoo 奇摩股市）獲取股票的最新價格。
//! 4. **邊界檢查**：判斷最新價格是否低於設定的最低價（Floor）或超過最高價（Ceiling）。
//! 5. **頻率限制**：利用 Redis 記錄已發送過的提醒，避免在短時間內重複發送相同的警報。
//! 6. **發送通知**：透過 Telegram Bot 將警報訊息傳送給使用者。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use chrono::{Datelike, Local, NaiveDate};
use futures::future;
use rust_decimal::Decimal;
use tokio::{task, time};

use crate::bot::telegram::Telegram;
use crate::{
    bot,
    cache::SHARE,
    crawler::{self, twse},
    database::table::trace::Trace,
    declare, logging, nosql,
    util::{datetime::Weekend, map::Keyable},
};

/// 確保 `trace_price_run` 只有一個實例在執行。
static IS_RUNNING: AtomicBool = AtomicBool::new(false);

/// 執行股票價格追蹤任務的入口點。
///
/// 此函式會先進行基本的檢查（是否為週末或假日），如果符合追蹤條件，
/// 則會啟動一個非同步任務 `trace_price_run` 來持續監控股價。
///
/// # Errors
///
/// 如果在檢查假期時發生資料庫或網路錯誤，將會回傳 `Err`。
pub async fn execute() -> Result<()> {
    let now = Local::now();

    // 週末不處理
    if now.is_weekend() {
        return Ok(());
    }

    // 檢查是否為國定假日休市
    if is_holiday(now.date_naive()).await? {
        return Ok(());
    }

    // 檢查是否已經在運行，避免重複啟動
    if IS_RUNNING.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        logging::debug_file_async("股票追蹤任務已在運行中，跳過重複啟動".to_string());
        return Ok(());
    }

    // 啟動背景監控任務
    task::spawn(async move {
        trace_price_run().await;
        IS_RUNNING.store(false, Ordering::SeqCst);
    });

    Ok(())
}

/// 核心追蹤迴圈。
///
/// 此任務會在開盤期間每 30 秒執行一次 `trace_target_price`。
/// 當市場關閉（`is_open` 回傳 false）時，迴圈會終止。
async fn trace_price_run() {
    let mut ticker = time::interval(Duration::from_secs(30));

    loop {
        // 檢查是否在開盤時間內
        if !declare::StockExchange::TWSE.is_open() {
            logging::debug_file_async("已達關盤時間，停止追蹤任務".to_string());
            break;
        }

        // 執行目標價格檢查
        if let Err(why) = trace_target_price().await {
            logging::error_file_async(format!("Failed to trace target price: {:?}", why));
        }

        // 等待下一個間隔
        ticker.tick().await;
    }
}

/// 判斷特定日期是否為台灣證券交易所（TWSE）公告的休假日。
async fn is_holiday(today: NaiveDate) -> Result<bool> {
    let holidays = match twse::holiday_schedule::visit(today.year()).await {
        Ok(result) => result,
        Err(err) => {
            anyhow::bail!("Failed to visit TWSE holiday schedule: {:?}", err);
        }
    };

    for holiday in holidays {
        if holiday.date == today {
            logging::info_file_async(format!(
                "Today is a holiday ({}), and the market is closed.",
                holiday.why
            ));
            return Ok(true);
        }
    }

    Ok(false)
}

/// 獲取所有需要追蹤的股票清單並併行處理。
///
/// 最佳化：先將相同的股票代號歸類，減少對同一支股票重複爬取價格的次數。
async fn trace_target_price() -> Result<()> {
    let all_targets = Trace::fetch().await?;
    if all_targets.is_empty() {
        return Ok(());
    }

    // 按股票代號分組
    let mut grouped_targets: HashMap<String, Vec<Trace>> = HashMap::new();
    for target in all_targets {
        grouped_targets
            .entry(target.stock_symbol.clone())
            .or_default()
            .push(target);
    }

    let futures = grouped_targets
        .into_iter()
        .map(|(symbol, targets)| task::spawn(process_grouped_targets(symbol, targets)))
        .collect::<Vec<_>>();

    // 等待所有處理任務完成
    future::join_all(futures).await;

    Ok(())
}

/// 處理同一支股票的多個追蹤目標。
///
/// 1. 從遠端獲取一次目前報價。
/// 2. 若價格有效（非零），則檢查該股票的所有追蹤目標是否觸發警報。
async fn process_grouped_targets(symbol: String, targets: Vec<Trace>) {
    match crawler::fetch_stock_price_from_remote_site(&symbol).await {
        Ok(current_price) if current_price != Decimal::ZERO => {
            for target in targets {
                if let Err(why) = alert_on_price_boundary(target, current_price).await {
                    logging::error_file_async(format!("Error alerting for {}: {:?}", symbol, why));
                }
            }
        }
        Ok(_) => {
            logging::debug_file_async(format!("Stock {} current price is zero, skipping", symbol));
        }
        Err(why) => logging::error_file_async(format!("Failed to fetch price for {}: {:?}", symbol, why)),
    }
}

/// 判斷股價是否觸發警報，並在必要時發送通知。
///
/// 最佳化：快取 Key 改為 `symbol:boundary_type`，避免價位變動時每分鐘重複警報。
async fn alert_on_price_boundary(target: Trace, current_price: Decimal) -> Result<bool> {
    // 判斷當前價格是否在預定範圍內（如果在範圍內則不需提醒）
    if is_within_boundary(&target, current_price) {
        return Ok(false);
    }

    // 判定是觸發高標還是低標
    let boundary_type = if current_price < target.floor && target.floor > Decimal::ZERO {
        "floor"
    } else if current_price > target.ceiling && target.ceiling > Decimal::ZERO {
        "ceiling"
    } else {
        // 理論上不會走到這裡，因為 above implies !is_within_boundary
        return Ok(false);
    };

    // 檢查 Redis 快取，避免針對同一方向重複通知
    // Key 格式包含股票代號與邊界類型，存活時間設為 1 小時，避免頻繁轟炸
    let target_key = format!("{}:{}", target.key_with_prefix(), boundary_type);
    if let Ok(exist) = nosql::redis::CLIENT.contains_key(&target_key).await {
        if exist {
            return Ok(false);
        }
    }

    // 格式化訊息並發送
    let to_bot_msg = format_alert_message(&target, current_price).await;

    // 寫入快取 (有效期限 1 小時)
    if let Err(why) = nosql::redis::CLIENT
        .set(&target_key, current_price.to_string(), 60 * 60)
        .await {
            logging::error_file_async(format!("Failed to set Redis key {}: {:?}", target_key, why));
        }

    // 發送 Telegram 訊息
    bot::telegram::send(&to_bot_msg).await;

    Ok(true)
}

/// 格式化警報訊息內容。
async fn format_alert_message(target: &Trace, current_price: Decimal) -> String {
    let stock_name = SHARE
        .get_stock(&target.stock_symbol)
        .await
        .map_or_else(String::new, |stock| stock.name);

    let (boundary, limit) = if current_price < target.floor && target.floor > Decimal::ZERO {
        ("低於最低價", target.floor)
    } else {
        ("超過最高價", target.ceiling)
    };

    let escaped_name = Telegram::escape_markdown_v2(stock_name);
    let escaped_boundary = Telegram::escape_markdown_v2(boundary.to_string());
    let escaped_limit = Telegram::escape_markdown_v2(limit.to_string());
    let escaped_price = Telegram::escape_markdown_v2(current_price.to_string());
    let symbol = &target.stock_symbol;

    format!("{escaped_name} {escaped_boundary}:{escaped_limit}，目前報價:{escaped_price} [Yahoo 股市](https://tw\\.stock\\.yahoo\\.com/quote/{symbol})")
}

/// 判斷當前價格是否在預定的 [floor, ceiling] 範圍內。
///
/// 如果設定值為 0，表示不限制該方向的邊界。
fn is_within_boundary(target: &Trace, current_price: Decimal) -> bool {
    let floor = target.floor;
    let ceiling = target.ceiling;

    match (floor > Decimal::ZERO, ceiling > Decimal::ZERO) {
        (true, true) => current_price >= floor && current_price <= ceiling,
        (true, false) => current_price >= floor,
        (false, true) => current_price <= ceiling,
        _ => true, // 如果都沒設定，視為在範圍內
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    #[test]
    fn test_is_within_boundary() {
        // 設定高低標 (500 ~ 600)
        let mut trace = Trace {
            stock_symbol: "2330".to_string(),
            floor: dec!(500),
            ceiling: dec!(600),
        };

        // 邊界測試
        assert!(is_within_boundary(&trace, dec!(550)));
        assert!(is_within_boundary(&trace, dec!(500)));
        assert!(is_within_boundary(&trace, dec!(600)));
        assert!(!is_within_boundary(&trace, dec!(499.9)));
        assert!(!is_within_boundary(&trace, dec!(600.1)));

        // 僅設定低標 (>= 500)
        trace.ceiling = Decimal::ZERO;
        assert!(is_within_boundary(&trace, dec!(500)));
        assert!(is_within_boundary(&trace, dec!(1000)));
        assert!(!is_within_boundary(&trace, dec!(499.9)));

        // 僅設定高標 (<= 600)
        trace.floor = Decimal::ZERO;
        trace.ceiling = dec!(600);
        assert!(is_within_boundary(&trace, dec!(600)));
        assert!(is_within_boundary(&trace, dec!(0.1)));
        assert!(!is_within_boundary(&trace, dec!(600.1)));

        // 皆未設定
        trace.ceiling = Decimal::ZERO;
        assert!(is_within_boundary(&trace, dec!(123)));
    }

    #[tokio::test]
    #[ignore]
    async fn test_format_alert_message() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        let trace = Trace {
            stock_symbol: "2330".to_string(),
            floor: dec!(500),
            ceiling: dec!(600),
        };

        // 觸發高標
        let msg = format_alert_message(&trace, dec!(650)).await;
        assert!(msg.contains("超過最高價"));
        assert!(msg.contains("目前報價:650"));

        // 觸發低標
        let msg = msg_low(&trace, dec!(450)).await;
        assert!(msg.contains("低於最低價"));
        assert!(msg.contains("目前報價:450"));
    }

    async fn msg_low(target: &Trace, price: Decimal) -> String {
        format_alert_message(target, price).await
    }

    #[tokio::test]
    #[ignore]
    async fn test_handle_price() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        let trace = Trace {
            stock_symbol: "1303".to_string(),
            floor: dec!(70),
            ceiling: dec!(60),
        };

        let result = alert_on_price_boundary(trace, dec!(560)).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[ignore]
    async fn test_trace_stock_price() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        let result = trace_target_price().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        let result = execute().await;
        assert!(result.is_ok());
    }
}

