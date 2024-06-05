use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{Datelike, Local, NaiveDate};
use futures::future;
use rust_decimal::Decimal;
use tokio::{task, time, time::Instant};

use crate::{
    bot,
    cache::SHARE,
    crawler::{self, twse},
    database::table::trace::Trace,
    declare, logging, nosql,
    util::{datetime::Weekend, map::Keyable},
};

/// 提醒本日已達高低標的股票有那些
pub async fn execute() -> Result<()> {
    let now = Local::now();

    if now.is_weekend() {
        return Ok(());
    }

    // 檢查是否為國定假日休市
    if is_holiday(now.date_naive()).await? {
        return Ok(());
    }

    task::spawn(async {
        let mut task_interval = time::interval_at(Instant::now(), Duration::from_secs(60));
        loop {
            task_interval.tick().await;
            // 檢查是否在開盤時間內
            if !declare::StockExchange::TWSE.is_open() {
                logging::debug_file_async("已達關盤時間".to_string());
                break;
            }

            if let Err(why) = trace_target_price().await {
                logging::error_file_async(format!("Failed to trace target price: {:?}", why));
            }
        }
    });

    Ok(())
}

/// 檢查給定日期是否為假日
async fn is_holiday(today: NaiveDate) -> Result<bool> {
    let holidays = twse::holiday_schedule::visit(today.year())
        .await
        .context("Failed to visit TWSE holiday schedule")?;

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

async fn trace_target_price() -> Result<()> {
    let futures = Trace::fetch()
        .await?
        .into_iter()
        .map(|target| task::spawn(process_target_price(target)))
        .collect::<Vec<_>>();

    future::join_all(futures).await;

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

    // 與快取中的價格比較，判斷是否需要傳送警告
    if no_need_to_alert(&target, current_price) {
        return Ok(false);
    }

    let target_key = format!("{}={}", target.key_with_prefix(), current_price);
    if let Ok(exist) = nosql::redis::CLIENT.contains_key(&target_key).await {
        if exist {
            return Ok(false);
        }
    }

    let to_bot_msg = format_alert_message(&target, current_price).await;

    nosql::redis::CLIENT
        .set(target_key, current_price.to_string(), 60 * 60 * 5)
        .await?;

    bot::telegram::send(&to_bot_msg).await?;

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
    let floor = target.floor;
    let ceiling = target.ceiling;

    match (floor > Decimal::ZERO, ceiling > Decimal::ZERO) {
        (true, true) => current_price >= floor && current_price <= ceiling,
        (true, false) => current_price >= floor,
        (false, true) => current_price <= ceiling,
        _ => false,
    }
}

fn no_need_to_alert(target: &Trace, current_price: Decimal) -> bool {
    if target.floor > Decimal::ZERO && target.ceiling > Decimal::ZERO {
        return current_price >= target.floor && current_price <= target.ceiling;
    }

    if target.floor > Decimal::ZERO {
        return current_price > target.floor;
    }

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
