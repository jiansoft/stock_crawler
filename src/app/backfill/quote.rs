use std::{future::Future, time::Duration};

use anyhow::Result;
use chrono::NaiveDate;
use futures::{StreamExt, stream};

use crate::{
    app::backfill::acl::QuoteAclMapper,
    core::logging,
    core::util::{self, map::Keyable},
    domain::quote::repository::QuoteRepository,
    infra::cache::{SHARE, TTL, TtlCacheInner},
    infra::crawler::{share::DailyQuoteDto, tpex, twse},
};

/// 調用  twse、tpex API 取得台股收盤報價
pub async fn execute(date: NaiveDate) -> Result<usize> {
    //上市報價
    let twse = twse::quote::visit(date);
    //上櫃報價
    let tpex = tpex::quote::visit(date);
    let mut quotes_twse: Vec<DailyQuoteDto> = Vec::with_capacity(1024);
    let mut quotes_tpex: Vec<DailyQuoteDto> = Vec::with_capacity(1024);
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
        // 實例化系統設定領域倉儲，用來查詢與更新最後收盤日設定
        let config_repo = crate::infra::database::repository::config::PgConfigRepository::new();
        use crate::domain::config::entity::SystemConfig;
        use crate::domain::config::repository::ConfigRepository;

        // 取得資料庫中現存的最後收盤日設定
        let config_opt = config_repo.find_by_key("last-closing-day").await?;
        let should_save = match &config_opt {
            Some(cfg) => cfg.should_update_date(date),
            None => true, // 若無設定則必須寫入
        };

        // 只有在需要更新時（新日期大於已存在日期，或尚無設定時）才儲存
        if should_save {
            let new_config = SystemConfig::new(
                "last-closing-day".to_string(),
                date.format("%Y-%m-%d").to_string(),
            );
            config_repo.save(&new_config).await?;
        }
        logging::info_file_async("最後收盤日設定更新到資料庫完成".to_string());
    }

    Ok(quotes_len)
}

/// 從指定資料來源抓取收盤價並附加到輸出向量。
pub async fn get_quotes_from_source(
    source: impl Future<Output = Result<Vec<DailyQuoteDto>>>,
    source_name: &str,
    quotes: &mut Vec<DailyQuoteDto>,
) -> Result<()> {
    if let Ok(quote) = source.await {
        quotes.extend(quote);
        logging::info_file_async(format!("取完{}收盤數據", source_name));
    }
    Ok(())
}

/// 將收盤價整批寫入資料庫並更新主快取。
pub async fn process_quotes(quotes: Vec<DailyQuoteDto>) {
    // 將 DTO 轉換為指令對象
    let cmds: Vec<_> = quotes.iter().map(QuoteAclMapper::from_dto).collect();
    // 直接將指令對象轉換為領域層的每日報價實體
    let domain_entities: Vec<crate::domain::quote::entity::DailyQuote> =
        cmds.iter().map(QuoteAclMapper::from_command).collect();

    // 實例化報價領域的倉儲
    let repo = crate::infra::database::repository::quote::PgQuoteRepository::new();
    // 呼叫倉儲的批次寫入合約，將資料寫入 PostgreSQL
    let result_count = match repo.batch_save_daily_quotes(&domain_entities).await {
        Ok(_) => domain_entities.len(),
        Err(why) => {
            logging::error_file_async(format!("Failed to batch save daily quotes: {:?}", why));
            0
        }
    };

    // 併行處理快取寫入與更新
    stream::iter(domain_entities)
        .for_each_concurrent(util::concurrent_limit_32(), |dq| async move {
            process_daily_quote(dq).await;
        })
        .await;
    logging::info_file_async(format!("上市櫃收盤數據更新到資料庫完成: {}", result_count));
}

async fn process_daily_quote(daily_quote: crate::domain::quote::entity::DailyQuote) {
    // 將最新報價更新至全域記憶體快取
    SHARE.set_stock_last_price(&daily_quote).await;

    // 取得記憶體快取之 key 值
    let daily_quote_memory_key = daily_quote.key_with_prefix();

    // 更新最後交易日的收盤價 TTL 快取（24 小時失效時間）
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
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    //use crossbeam::thread;
    use rayon::prelude::*;
    use tokio::time::sleep;

    use crate::{core::logging, infra::cache::SHARE};

    use super::*;

    //use std::time;

    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 execute".to_string());
        //let date = Local::now().date_naive();
        let date = NaiveDate::from_ymd_opt(2026, 4, 30).unwrap();
        let quote_repo = crate::infra::database::repository::quote::PgQuoteRepository::new();
        let _ = quote_repo.delete_quotes_by_date(date).await;

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
    #[ignore]
    async fn test_thread() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 execute".to_string());

        let stock_repo = crate::infra::database::repository::stock::PgStockRepository::new();
        use crate::domain::registry::repository::StockRepository;
        let stocks = stock_repo.fetch_all_active().await.unwrap();
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

    fn calculate_day_quotes_moving_average_worker(
        i: usize,
        dq: &crate::domain::registry::entity::Stock,
    ) {
        logging::debug_file_async(format!("dq[{}]:{:?}", i, dq));
    }
}
