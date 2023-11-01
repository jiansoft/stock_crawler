use std::time::Duration;

use anyhow::Result;
use chrono::Local;
use futures::{stream, StreamExt};

use crate::{
    cache::{TtlCacheInner, SHARE, TTL},
    crawler::{tpex, twse},
    database::table::{self, daily_quote},
    logging, util,
};

/// 調用  twse、tpex API 取得台股收盤報價
pub async fn execute() -> Result<usize> {
    let now = Local::now();
    let mut quotes: Vec<daily_quote::DailyQuote> = Vec::with_capacity(2048);
    //上市報價
    let twse = twse::quote::visit(now);
    //上櫃報價
    let tpex = tpex::quote::visit(now);
    let (res_twse, res_tpex) = tokio::join!(twse, tpex);

    if let Ok(quote) = res_twse {
        quotes.extend(quote);
        logging::info_file_async("取完上市收盤數據".to_string());
    }

    if let Ok(quote) = res_tpex {
        quotes.extend(quote);
        logging::info_file_async("取完上櫃收盤數據".to_string());
    }

    let quotes_len = quotes.len();

    if quotes_len > 0 {
        stream::iter(quotes)
            .for_each_concurrent(util::concurrent_limit_32(), |dq| async move {
                process_daily_quote(dq).await;
            })
            .await;

        logging::info_file_async("上市櫃收盤數據更新到資料庫完成".to_string());

        let last_closing_day_config = table::config::Config::new(
            "last-closing-day".to_string(),
            now.date_naive().format("%Y-%m-%d").to_string(),
        );

        last_closing_day_config.set_val_as_naive_date().await?;
        logging::info_file_async("最後收盤日設定更新到資料庫完成".to_string());
    }

    Ok(quotes_len)
}

async fn process_daily_quote(daily_quote: daily_quote::DailyQuote) {
    match daily_quote.upsert().await {
        Ok(_) => {
            SHARE.set_stock_last_price(&daily_quote).await;

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
            logging::error_file_async(format!("({:#?}", why));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc
        }
    };

    //use crossbeam::thread;
    use rayon::prelude::*;
    use tokio::time::sleep;

    use crate::{cache::SHARE, database::table::stock, logging};

    use super::*;

    //use std::time;

    #[tokio::test]
    #[ignore]
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
