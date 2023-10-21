use std::{fmt::Write, time::Duration};

use anyhow::{anyhow, Result};
use chrono::{Local, Timelike};
use rust_decimal::Decimal;
use tokio::{time, time::Instant};

use crate::internal::crawler::{cmoney, histock};
use crate::internal::{
    bot,
    cache::{TtlCacheInner, SHARE, TTL},
    crawler::{cnyes, megatime, yahoo},
    database::table::trace::Trace,
    logging,
    util::datetime::Weekend,
};

/// 提醒本日已達高低標的股票有那些
pub async fn execute() -> Result<()> {
    if Local::now().is_weekend() {
        return Ok(());
    }
    //加十秒後再執行，確保已有交易資料
    let start = Instant::now() + Duration::from_secs(10);
    let interval = Duration::from_secs(60);
    let mut task_interval = time::interval_at(start, interval);

    loop {
        task_interval.tick().await;

        let now = Local::now();
        // 檢查當前時間是否還未到九點與是否超過13:30 關盤時間
        if now.hour() < 9 || (now.hour() > 13 || (now.hour() == 13 && now.minute() >= 30)) {
            logging::debug_file_async("已達關盤時間".to_string());
            break;
        }

        logging::debug_file_async("開始追踪股價".to_string());
        if let Err(why) = trace_price().await {
            logging::error_file_async(format!("{:?}", why));
        }
        logging::debug_file_async("結束追踪股價".to_string());
    }

    Ok(())
}

async fn trace_price() -> Result<()> {
    let futures = Trace::fetch()
        .await?
        .into_iter()
        .map(process_target)
        .collect::<Vec<_>>();
    futures::future::join_all(futures).await;

    Ok(())
}

async fn process_target(target: Trace) {
    let target_key = format!("trace_quote:{}", target.key());
    if TTL.trace_quote_contains_key(&target_key) {
        return;
    }

    match fetch_stock_price_from_remote_site(&target.stock_symbol).await {
        Ok(current_price) if current_price != Decimal::ZERO => {
            match alert_on_price_boundary(target, current_price).await {
                Ok(is_alert) => {
                    if is_alert {
                        TTL.trace_quote_set(target_key, true, Duration::from_secs(60 * 60 * 5));
                    }
                }
                Err(why) => {
                    logging::error_file_async(format!("{:?}", why));
                }
            }
        }
        Ok(_) => {}
        Err(why) => logging::error_file_async(format!("{:?}", why)),
    }
}

async fn alert_on_price_boundary(target: Trace, price: Decimal) -> Result<bool> {
    if (price < target.floor && target.floor > Decimal::ZERO)
        || (price > target.ceiling && target.ceiling > Decimal::ZERO)
    {
        let mut to_bot_msg = String::with_capacity(64);
        let stock_cache = SHARE.get_stock(&target.stock_symbol).await;
        let stock_name = match stock_cache {
            None => String::from(""),
            Some(stock) => stock.name,
        };

        let boundary = if price < target.floor {
            "低於最低價"
        } else {
            "超過最高價"
        };
        let limit = if price < target.floor {
            target.floor
        } else {
            target.ceiling
        };

        let _ = writeln!(&mut to_bot_msg, "{stock_name} {boundary}:{limit}，目前報價:{price} https://tw.stock.yahoo.com/quote/{stock_symbol}",
                         boundary = boundary, limit = limit, price = price, stock_symbol = target.stock_symbol, stock_name = stock_name);

        if !to_bot_msg.is_empty() {
            if let Err(why) = bot::telegram::send(&to_bot_msg).await {
                logging::error_file_async(format!("Failed to send because {:?}", why));
            }

            return Ok(true);
        }
    }

    Ok(false)
}

pub async fn fetch_stock_price_from_remote_site(stock_symbol: &str) -> Result<Decimal> {
    let price = yahoo::price::get(stock_symbol).await;
    if price.is_ok() {
        return price;
    }

    let price = megatime::price::get(stock_symbol).await;
    if price.is_ok() {
        return price;
    }

    let price = cnyes::price::get(stock_symbol).await;
    if price.is_ok() {
        return price;
    }

    let price = histock::price::get(stock_symbol).await;
    if price.is_ok() {
        return price;
    }

    let price = cmoney::price::get(stock_symbol).await;
    if price.is_ok() {
        return price;
    }

    Err(anyhow!("Failed to fetch stock price from all sites"))
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
                    "event::trace::stock_price::handle_price 完成".to_string(),
                );
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to event::taiwan_stock::closing::aggregate because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 event::trace::stock_price::handle_price".to_string());
    }

    #[tokio::test]
    async fn test_fetch_price() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_price".to_string());

        match fetch_stock_price_from_remote_site("2330").await {
            Ok(e) => {
                dbg!(e);
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to fetch_price because {:?}", why));
            }
        }

        logging::debug_file_async("結束 fetch_price".to_string());
    }

    #[tokio::test]
    async fn test_trace_stock_price() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 trace_stock_price".to_string());

        let _ = trace_price().await;

        logging::debug_file_async("結束 trace_stock_price".to_string());
    }
}
