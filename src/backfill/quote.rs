use std::{future::Future, time::Duration};

use anyhow::Result;
use chrono::NaiveDate;
use futures::{stream, StreamExt};

use crate::{
    cache::{TtlCacheInner, SHARE, TTL},
    crawler::{tpex, twse},
    database::table::{self, daily_quote::DailyQuote},
    logging, util,
    util::map::Keyable,
};

/// 調用  twse、tpex API 取得台股收盤報價
pub async fn execute(date: NaiveDate) -> Result<usize> {
    //上市報價
    let twse = twse::quote::visit(date);
    //上櫃報價
    let tpex = tpex::quote::visit(date);
    let mut quotes_twse: Vec<DailyQuote> = Vec::with_capacity(1024);
    let mut quotes_tpex: Vec<DailyQuote> = Vec::with_capacity(1024);
    let get_twse = get_quotes_from_source(twse, "上市", &mut quotes_twse);
    let get_tpex = get_quotes_from_source(tpex, "上櫃", &mut quotes_tpex);
    let (result_twse, result_tpex) = tokio::join!(get_twse, get_tpex);

    result_twse?;
    result_tpex?;

    let quotes_len = quotes_twse.len() + quotes_tpex.len();
    let mut quotes = Vec::with_capacity(quotes_len);

    quotes.append(&mut quotes_twse);
    quotes.append(&mut quotes_tpex);

    if quotes_len > 0 {
        process_quotes(quotes).await;
        let last_closing_day_config = table::config::Config::new(
            "last-closing-day".to_string(),
            date.format("%Y-%m-%d").to_string(),
        );

        last_closing_day_config.set_val_as_naive_date().await?;
        logging::info_file_async("最後收盤日設定更新到資料庫完成".to_string());
    }

    Ok(quotes_len)
}

pub async fn get_quotes_from_source(
    source: impl Future<Output = Result<Vec<DailyQuote>>>,
    source_name: &str,
    quotes: &mut Vec<DailyQuote>,
) -> Result<()> {
    if let Ok(quote) = source.await {
        quotes.extend(quote);
        logging::info_file_async(format!("取完{}收盤數據", source_name));
    }
    Ok(())
}

pub async fn process_quotes(quotes: Vec<DailyQuote>) {
    let result_count = DailyQuote::copy_in_raw(&quotes).await.unwrap_or_default();
    stream::iter(quotes)
        .for_each_concurrent(util::concurrent_limit_32(), |dq| async move {
            process_daily_quote(dq).await;
        })
        .await;
    logging::info_file_async(format!("上市櫃收盤數據更新到資料庫完成: {}", result_count));
}

async fn process_daily_quote(daily_quote: DailyQuote) {
    SHARE.set_stock_last_price(&daily_quote).await;

    let daily_quote_memory_key = daily_quote.key_with_prefix();

    //更新最後交易日的收盤價
    TTL.daily_quote_set(
        daily_quote_memory_key,
        "".to_string(),
        Duration::from_secs(60 * 60 * 24),
    );
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    //use crossbeam::thread;
    use rayon::prelude::*;
    use tokio::time::sleep;

    use crate::{cache::SHARE, database, database::table::stock, logging};

    use super::*;

    //use std::time;

    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 execute".to_string());
        //let date = Local::now().date_naive();
        let date = NaiveDate::from_ymd_opt(2024, 12, 6).unwrap();
        let _ = sqlx::query(r#"delete from "DailyQuotes" where "Date" = $1;"#)
            .bind(date)
            .execute(database::get_connection())
            .await;

        match execute(date).await {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!("Failed to execute because {:?}", why));
            }
        }

        logging::debug_file_async("結束 execute".to_string());
        sleep(Duration::from_secs(1)).await;
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
