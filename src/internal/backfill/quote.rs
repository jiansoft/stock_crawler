use crate::internal::{
    cache::{TtlCacheInner, SHARE, TTL},
    crawler::{tpex, twse},
    database::table::{self, daily_quote},
    logging,
};
use anyhow::*;
use chrono::{Local, NaiveDate};
use core::result::Result::Ok;
use std::time::Duration;

/// 調用  twse API 取得台股收盤報價
pub async fn execute() -> Result<()> {
    let now = Local::now();
    let mut results: Vec<daily_quote::DailyQuote> = Vec::with_capacity(2048);

    //上市報價
    if let Ok(twse) = twse::quote::visit(now).await {
        results.extend(twse);
    }

    //上櫃報價
    if let Ok(tpex) = tpex::quote::visit(now).await {
        results.extend(tpex);
    }

    let results_is_empty = results.is_empty();

    let tasks: Vec<_> = results.into_iter().map(process_daily_quote).collect();
    futures::future::join_all(tasks).await;

    if results_is_empty {
        return Ok(());
    }

    let date_naive = now.date_naive();
    match daily_quote::makeup_for_the_lack_daily_quotes(date_naive).await {
        Ok(result) => {
            logging::info_file_async(format!("補上當日缺少的每日收盤數據結束:{:#?}", result));
        }
        Err(why) => {
            logging::error_file_async(format!(
                "Failed to makeup_for_the_lack_daily_quotes because:{:?}",
                why
            ));
        }
    };

    if let Ok(c) = table::config::Entity::first("last-closing-day").await {
        let date = NaiveDate::parse_from_str(&c.val, "%Y-%m-%d")?;
        if date_naive > date {
            let mut new_c = table::config::Entity::new(c.key);
            new_c.val = date_naive.format("%Y-%m-%d").to_string();
            match new_c.upsert().await {
                Ok(_) => {}
                Err(why) => {
                    logging::error_file_async(format!("Failed to config.upsert because:{:?}", why));
                }
            }
        }
    }

    Ok(())
}

async fn process_daily_quote(daily_quote: daily_quote::DailyQuote) {
    match daily_quote.upsert().await {
        Ok(_) => {
            //logging::debug_file_async(format!("item:{:#?}", item));

            if let Ok(mut last_trading_day_quotes) = SHARE.last_trading_day_quotes.write() {
                if let Some(quote) = last_trading_day_quotes.get_mut(&daily_quote.security_code) {
                    quote.date = daily_quote.date;
                    quote.closing_price = daily_quote.closing_price;
                }
            }

            let daily_quote_memory_key = format!(
                "DailyQuote:{}-{}",
                daily_quote.date.format("%Y%m%d"),
                daily_quote.security_code
            );

            //更新最後交易日的收盤價
            TTL.daily_quote_set(
                daily_quote_memory_key,
                "".to_string(),
                Duration::from_secs(60 * 60 * 24),
            );
        }
        Err(why) => {
            logging::error_file_async(format!("Failed to quote.upsert because {:?}", why));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::cache::SHARE;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    //use std::time;

    use crate::internal::database::table::stock;
    use crate::internal::logging;
    //use crossbeam::thread;
    use rayon::prelude::*;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_execute() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 execute".to_string());

        match execute().await {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!("Failed to execute because {:?}", why));
            }
        }

        logging::debug_file_async("結束 execute".to_string());
    }

    #[tokio::test]
    async fn test_thread() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 execute".to_string());

        let stocks = stock::Stock::fetch().await.unwrap();
        let worker_count = num_cpus::get() * 100;
        //let dqs_arc = Arc::new(stocks);
        let counter = Arc::new(AtomicUsize::new(0));
        logging::debug_file_async(format!("stocks:{}", stocks.len()));
        /*  thread::scope(|scope| {
            for i in 0..worker_count {
                let dqs = Arc::clone(&dqs_arc);
                let counter = Arc::clone(&counter);

                scope.spawn(move |_| {
                    calculate_day_quotes_moving_average_worker(i, dqs.to_vec(), &counter);
                });
            }
        })
        .unwrap();*/
        stocks
            .par_iter()
            .with_min_len(worker_count)
            .for_each(|stock| {
                let index = counter.fetch_add(1, Ordering::SeqCst);
                calculate_day_quotes_moving_average_worker(index, stock);
            });

        logging::debug_file_async("結束 execute".to_string());
        sleep(Duration::from_secs(1)).await;
    }

    fn calculate_day_quotes_moving_average_worker(i: usize, dq: &stock::Stock) {
        logging::debug_file_async(format!("dq[{}]:{:?}", i, dq));
    }
}
