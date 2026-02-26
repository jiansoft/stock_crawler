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

    // 啟動背景監控任務
    task::spawn(trace_price_run());

    Ok(())
}

/// 核心追蹤迴圈。
///
/// 此任務會在開盤期間每 60 秒執行一次 `trace_target_price`。
/// 當市場關閉（`is_open` 回傳 false）時，迴圈會終止。
async fn trace_price_run() {
    let mut ticker = time::interval(Duration::from_secs(60));

    loop {
        // 檢查是否在開盤時間內
        if !declare::StockExchange::TWSE.is_open() {
            logging::debug_file_async("已達關盤時間".to_string());
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
///
/// # 參數
/// * `today` - 要檢查的日期。
///
/// # 回傳
/// * `Ok(true)` - 如果是休假日。
/// * `Ok(false)` - 如果是交易日。
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
/// 此函式會從資料庫中讀取所有 `Trace` 記錄，並為每一筆記錄建立一個新的任務進行處理。
async fn trace_target_price() -> Result<()> {
    let futures = Trace::fetch()
        .await?
        .into_iter()
        .map(|target| task::spawn(process_target_price(target)))
        .collect::<Vec<_>>();

    // 等待所有處理任務完成
    future::join_all(futures).await;

    Ok(())
}

/// 處理單一追蹤目標的價格檢查。
///
/// 1. 從遠端獲取目前報價。
/// 2. 若價格有效（非零），則檢查是否觸發警報條件。
async fn process_target_price(target: Trace) {
    match crawler::fetch_stock_price_from_remote_site(&target.stock_symbol).await {
        Ok(current_price) if current_price != Decimal::ZERO => {
            if let Err(why) = alert_on_price_boundary(target, current_price).await {
                logging::error_file_async(format!("{:?}", why));
            }
        }
        Ok(_) => {}
        Err(why) => logging::error_file_async(format!("{:?}", why)),
    }
}

/// 判斷股價是否觸發警報，並在必要時發送通知。
///
/// 此函式結合了邊界檢查與 Redis 快取機制，確保：
/// 1. 只有在價格超出範圍時才發送提醒。
/// 2. 相同的價格點或短時間內不會重複發送提醒。
///
/// # 參數
/// * `target` - 包含追蹤代碼、最低價與最高價設定的對象。
/// * `current_price` - 目前獲取到的即時報價。
///
/// # 回傳
/// * `Ok(true)` - 已發送提醒。
/// * `Ok(false)` - 未發送提醒（價格在範圍內、或已在快取中）。
async fn alert_on_price_boundary(target: Trace, current_price: Decimal) -> Result<bool> {
    // 判斷當前價格是否在預定範圍內（如果在範圍內則不需提醒）
    if within_boundary(&target, current_price) {
        return Ok(false);
    }

    // 進一步確認是否真的需要提醒（邏輯互補）
    if no_need_to_alert(&target, current_price) {
        return Ok(false);
    }

    // 檢查 Redis 快取，避免重複通知
    // Key 格式包含股票代號與當前價格，存活時間約 5 小時
    let target_key = format!("{}={}", target.key_with_prefix(), current_price);
    if let Ok(exist) = nosql::redis::CLIENT.contains_key(&target_key).await {
        if exist {
            return Ok(false);
        }
    }

    // 格式化訊息並發送
    let to_bot_msg = format_alert_message(&target, current_price).await;

    // 寫入快取
    nosql::redis::CLIENT
        .set(target_key, current_price.to_string(), 60 * 60 * 5)
        .await?;

    // 發送 Telegram 訊息
    bot::telegram::send(&to_bot_msg).await;

    Ok(true)
}

/// 格式化警報訊息內容。
///
/// 訊息包含：股票名稱、警報類型（低於最低/超過最高）、設定限額、目前報價以及 Yahoo 股市連結。
async fn format_alert_message(target: &Trace, current_price: Decimal) -> String {
    let stock_name = SHARE
        .get_stock(&target.stock_symbol)
        .await
        .map_or_else(String::new, |stock| stock.name);

    let boundary = if current_price < target.floor {
        "低於最低價"
    } else {
        "超過最高價"
    };

    let limit = if current_price < target.floor {
        target.floor
    } else {
        target.ceiling
    };

    format!("{stock_name} {boundary}:{limit}，目前報價:{price} https://tw\\.stock\\.yahoo\\.com/quote/{stock_symbol}",
            boundary = Telegram::escape_markdown_v2(boundary.to_string()),
            limit = Telegram::escape_markdown_v2(limit.to_string()),
            price = Telegram::escape_markdown_v2(current_price.to_string()),
            stock_symbol = target.stock_symbol,
            stock_name = Telegram::escape_markdown_v2(stock_name))
}

/// 判斷當前價格是否在預定的 [floor, ceiling] 範圍內。
///
/// 如果設定值為 0，表示不限制該方向的邊界。
///
/// # 參數
/// - `target`: 包含 `floor` (最低價) 和 `ceiling` (最高價) 的 `Trace` 對象。
/// - `current_price`: 當前股價。
///
/// # 回傳
/// - `true`: 價格在安全範圍內（不觸發警報）。
/// - `false`: 價格已超出設定範圍。
fn within_boundary(target: &Trace, current_price: Decimal) -> bool {
    let floor = target.floor;
    let ceiling = target.ceiling;

    match (floor > Decimal::ZERO, ceiling > Decimal::ZERO) {
        (true, true) => current_price >= floor && current_price <= ceiling,
        (true, false) => current_price >= floor,
        (false, true) => current_price <= ceiling,
        _ => false,
    }
}

/// 輔助判斷是否不需要發送提醒。
///
/// 此函式的邏輯與 `within_boundary` 相似但有細微差別，主要用於雙重確認。
/// 如果任一條件不滿足（即價格超出設定值），則回傳 `false` 表示「需要提醒」。
fn no_need_to_alert(target: &Trace, current_price: Decimal) -> bool {
    // 同時設定了高低標
    if target.floor > Decimal::ZERO && target.ceiling > Decimal::ZERO {
        return current_price >= target.floor && current_price <= target.ceiling;
    }

    // 只設定了低標
    if target.floor > Decimal::ZERO {
        return current_price > target.floor;
    }

    // 只設定了高標
    if target.ceiling > Decimal::ZERO {
        return current_price < target.ceiling;
    }

    true
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_handle_price() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        logging::debug_file_async("開始 event::trace::stock_price::handle_price".to_string());

        let trace = Trace {
            stock_symbol: "1303".to_string(),
            floor: dec!(70),
            ceiling: dec!(60),
        };

        match alert_on_price_boundary(trace, dec!(560)).await {
            Ok(_) => {
                logging::debug_file_async(
                    "event::trace::stock_price::alert_on_price_boundary 完成".to_string(),
                );
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to event::trace::stock_price::alert_on_price_boundary because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async(
            "結束 event::trace::stock_price::alert_on_price_boundary".to_string(),
        );
    }

    #[tokio::test]
    #[ignore]
    async fn test_trace_stock_price() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 trace_stock_price".to_string());

        match trace_target_price().await {
            Ok(_) => {
                logging::debug_file_async("test_trace_stock_price 完成".to_string());
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to test_trace_stock_price because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 trace_stock_price".to_string());
    }

    #[tokio::test]
    #[ignore]
    async fn test_process_target_price() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        logging::debug_file_async(
            "開始 event::trace::stock_price::process_target_price".to_string(),
        );

        let trace = Trace {
            stock_symbol: "1558".to_string(),
            floor: dec!(100),
            ceiling: dec!(0),
        };

        process_target_price(trace).await;

        logging::debug_file_async(
            "結束 event::trace::stock_price::process_target_price".to_string(),
        );
    }

    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 execute".to_string());

        match execute().await {
            Ok(_) => {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                //dbg!(&list);
                //logging::debug_file_async(format!("list:{:#?}", list));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to execute because: {:?}", why));
            }
        }

        logging::debug_file_async("結束 execute".to_string());
    }
}
