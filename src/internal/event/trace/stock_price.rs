use std::time::Duration;

use anyhow::Result;
use chrono::{Local, Timelike};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tokio::{time, time::Instant};

use crate::{
    internal::{
        bot,
        cache::{TtlCacheInner, SHARE, TTL},
        crawler::{self},
        database::table::trace::Trace,
    },
    logging,
    util::{datetime::Weekend, map::Keyable},
};

/// 提醒本日已達高低標的股票有那些
pub async fn execute() -> Result<()> {
    if Local::now().is_weekend() {
        return Ok(());
    }

    let mut task_interval = time::interval_at(Instant::now(), Duration::from_secs(60));

    loop {
        task_interval.tick().await;

        let now = Local::now();
        // 檢查當前時間是否還未到九點與是否超過13:30 關盤時間
        if now.hour() < 9 || (now.hour() > 13 || (now.hour() == 13 && now.minute() >= 30)) {
            logging::debug_file_async("已達關盤時間".to_string());
            break;
        }

        if let Err(why) = trace_target_price().await {
            logging::error_file_async(format!("{:?}", why));
        }
    }

    Ok(())
}

async fn trace_target_price() -> Result<()> {
    let futures = Trace::fetch()
        .await?
        .into_iter()
        .map(process_target_price)
        .collect::<Vec<_>>();
    futures::future::join_all(futures).await;

    Ok(())
}

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

async fn alert_on_price_boundary(target: Trace, current_price: Decimal) -> Result<bool> {
    // 判斷當前價格是否在預定範圍內
    if within_boundary(&target, current_price) {
        return Ok(false);
    }

    let target_key = target.key().to_string();

    // 與快取中的價格比較，判斷是否需要傳送警告
    if let Some(last_price_in_cache) = TTL.trace_quote_get(&target_key) {
        if no_need_to_alert(&target, current_price, last_price_in_cache) {
            return Ok(false);
        }
    }

    let to_bot_msg = format_alert_message(&target, current_price).await;

    bot::telegram::send(&to_bot_msg).await?;

    TTL.trace_quote_set(target_key, current_price, Duration::from_secs(60 * 60 * 5));

    Ok(true)
}

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

    format!("{stock_name} {boundary}:{limit}，目前報價:{price} https://tw.stock.yahoo.com/quote/{stock_symbol}",
            boundary = boundary, limit = limit, price = current_price, stock_symbol = target.stock_symbol, stock_name = stock_name)
}


/// Checks whether the current price is within a specified boundary.
/// 判斷當前價格是否在預定範圍內
///
/// This function determines if the `current_price` is within a certain boundary specified
/// by the `target`. The boundary is defined by the `floor` and `ceiling` attributes of the
/// `target`. The function will return true under either of the following conditions:
/// - The `current_price` is greater than or equal to `target.floor`, and `target.floor` is
///   greater than zero.
/// - The `current_price` is less than or equal to `target.ceiling`, and `target.ceiling` is
///   greater than zero.
///
/// # Parameters
/// - `target`: A reference to a `Trace` object that contains the `floor` and `ceiling` values
///   that define the boundary.
/// - `current_price`: A `Decimal` value representing the current price to be checked against
///   the boundary.
///
/// # Returns
/// - Returns a boolean that is `true` if the `current_price` is within the boundary, and `false`
///   otherwise.
fn within_boundary(target: &Trace, current_price: Decimal) -> bool {
    (current_price >= target.floor && target.floor > Decimal::ZERO)
        || (current_price <= target.ceiling && target.ceiling > Decimal::ZERO)
}

/// 判斷是否不需要傳送警告
///
/// 這個函數會根據提供的目標值、當前價格和快取中的最後價格，來決定是否需要傳送警告。
/// 當滿足以下任一條件時，該函數將返回 true，表示不需要傳送警告：
/// - 如果當前價格大於或等於快取中的最後價格，並且目標的 floor 大於零。
/// - 如果當前價格小於或等於快取中的最後價格，並且目標的 ceiling 大於零。
///
/// # 參數
/// - `target`: 一個 `Trace` 引用，包含 floor 和 ceiling 的資訊。
/// - `current_price`: 一個 `Decimal` 類型，表示當前的價格。
/// - `last_price_in_cache`: 一個 `Decimal` 類型，表示快取中的最後價格。
///
/// # 返回
/// - 返回一個布林值，如果為 true，表示不需要傳送警告；否則，表示需要傳送警告。
fn no_need_to_alert(target: &Trace, current_price: Decimal, last_price_in_cache: Decimal) -> bool {
    (current_price >= last_price_in_cache && target.floor > Decimal::ZERO)
        || (current_price <= last_price_in_cache && Decimal::ZERO > target.ceiling)
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
            stock_symbol: "2330".to_string(),
            floor: dec!(520),
            ceiling: dec!(550),
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

        let _ = trace_target_price().await;

        logging::debug_file_async("結束 trace_stock_price".to_string());
    }
}
