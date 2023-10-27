use std::time::Duration;

use anyhow::Result;
use chrono::{Local, Timelike};
use rust_decimal::Decimal;
use tokio::{time, time::Instant};

use crate::internal::{
    bot,
    cache::{TtlCacheInner, SHARE, TTL},
    crawler::{self},
    database::table::trace::Trace,
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
    if (current_price < target.floor && target.floor > Decimal::ZERO)
        || (current_price > target.ceiling && target.ceiling > Decimal::ZERO)
    {
        let target_key = format!("{}-{}", target.key(), current_price);
        if TTL.trace_quote_contains_key(&target_key) {
            return Ok(false);
        }

        let to_bot_msg = format_alert_message(&target, current_price).await;

        if !to_bot_msg.is_empty() {
            bot::telegram::send(&to_bot_msg)
                .await
                .unwrap_or_else(|why| {
                    logging::error_file_async(format!("Failed to send message: {:?}", why));
                });
            TTL.trace_quote_set(target_key, true, Duration::from_secs(60 * 60 * 5));
            return Ok(true);
        }
    }

    Ok(false)
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

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    #[tokio::test]
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
    async fn test_trace_stock_price() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 trace_stock_price".to_string());

        let _ = trace_target_price().await;

        logging::debug_file_async("結束 trace_stock_price".to_string());
    }
}
