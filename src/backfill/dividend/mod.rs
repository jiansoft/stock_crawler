use std::{collections::HashSet, time::Duration};

use anyhow::{anyhow, Result};
use chrono::{Datelike, Local};
use tokio_retry::{
    strategy::{jitter, ExponentialBackoff},
    Retry,
};

use crate::{
    crawler::{goodinfo, yahoo},
    database::table::{self, dividend},
    logging, nosql,
    util::map::Keyable,
};


pub mod payout_ratio;

/// 更新股利發送數據
/// 資料庫內尚未有年度配息數據的股票取出後向第三方查詢後更新回資料庫
pub async fn execute() -> Result<()> {
    //尚未有股利或多次配息
    let now = Local::now();
    let year = now.year();
    let no_or_multiple_dividend = processing_no_or_multiple(year);
    let yahoo = processing_unannounced_ex_dividend_date(year);
    let (res_no_or_multiple, res_yahoo) = tokio::join!(no_or_multiple_dividend, yahoo);

    match res_no_or_multiple {
        Ok(_) => {
            logging::info_file_async(
                "processing_without_or_multiple executed successfully.".to_string(),
            );
        }
        Err(why) => {
            logging::error_file_async(format!(
                "Failed to process_without_or_multiple because {:?}",
                why
            ));
        }
    }

    match res_yahoo {
        Ok(_) => {
            logging::info_file_async(
                "processing_with_unannounced_ex_dividend_dates executed successfully.".to_string(),
            );
        }
        Err(why) => {
            logging::error_file_async(format!("Failed to process_yahoo because {:?}", why));
        }
    }

    Ok(())
}

/// Asynchronously processes the stocks that have no dividends or have issued multiple dividends.
///
/// This function fetches a list of stock symbols that either have no dividends or have issued multiple dividends
/// in the current year. It then visits the dividend information of each stock symbol using the `goodinfo::dividend::visit`
/// function. For each dividend found, if the dividend's year is not the current year, it skips to the next dividend.
/// Otherwise, it converts the dividend into a `table::dividend::Entity` and tries to upsert it.
///
/// If the upsert operation is successful, it logs the success and the entity that was upserted.
/// If the upsert operation fails, it logs the error.
///
/// Between each stock symbol processing, the function sleeps for 6 seconds to prevent too many requests to the
/// `goodinfo::dividend::visit` function.
///
/// Returns `Ok(())` if the function finishes processing all stock symbols. If any error occurs during the process,
/// it returns `Err(e)`, where `e` is the error.
///
/// # Errors
///
/// This function will return an error if:
/// - It fails to fetch the list of stock symbols.
/// - It fails to visit the dividend information of a stock symbol.
/// - It fails to upsert a dividend entity.
async fn processing_no_or_multiple(year: i32) -> Result<()> {
    //年度內尚未有股利配息資料
    let mut stock_symbols: HashSet<String> = dividend::Dividend::fetch_no_dividends_for_year(year)
        .await?
        .into_iter()
        .collect();
    //年度內有多次配息資料
    let multiple_dividends = dividend::Dividend::fetch_multiple_dividends_for_year(year).await?;
    let mut multiple_dividend_cache = HashSet::new();
    for dividend in multiple_dividends {
        let key = dividend.key();
        multiple_dividend_cache.insert(key);
        stock_symbols.insert(dividend.security_code.to_string());
    }

    logging::info_file_async(format!("本次殖利率的採集需收集 {} 家", stock_symbols.len()));
    for stock_symbol in stock_symbols {
        let cache_key = format!("goodinfo:dividend:{}", stock_symbol);
        let is_jump = nosql::redis::CLIENT.get_bool(&cache_key).await?;

        if is_jump {
            continue;
        }

        nosql::redis::CLIENT
            .set(cache_key, true, 60 * 60 * 24 * 3)
            .await?;

        if let Err(why) =
            process_stock_dividends(year, &stock_symbol, &multiple_dividend_cache).await
        {
            logging::error_file_async(format!("{:?} ", why));
        }

        tokio::time::sleep(Duration::from_secs(120)).await;
    }

    Ok(())
}

async fn process_stock_dividends(
    year: i32,
    stock_symbol: &str,
    multiple_dividend_cache: &HashSet<String>,
) -> Result<()> {
    let dividends_from_goodinfo = goodinfo::dividend::visit(stock_symbol).await?;
    let last_year = year - 1;
    let relevant_years = [year, last_year];
    // 合併今年度和去年的股利數據
    let dividend_details_from_goodinfo = relevant_years.iter().filter_map(|&yr| {
        dividends_from_goodinfo.get(&yr).map(|details| details.iter().cloned())
    }).flatten().collect::<Vec<_>>();

    for dividend_from_goodinfo in dividend_details_from_goodinfo {
        if dividend_from_goodinfo.year_of_dividend != year &&  dividend_from_goodinfo.year_of_dividend != last_year {
            continue;
        }

        //檢查是否為多次配息，並且已經收錄該筆股利
        let key = dividend_from_goodinfo.key();

        if multiple_dividend_cache.contains(&key) {
            continue;
        }

        let entity = table::dividend::Dividend::from(dividend_from_goodinfo);
        match entity.upsert().await {
            Ok(_) => {
                logging::debug_file_async(format!(
                    "dividend upsert executed successfully. \r\n{:#?}",
                    entity
                ));

                if !entity.quarter.is_empty() {
                    //更新股利年度的數據
                    if let Err(why) = entity.upsert_annual_total_dividend().await{
                        logging::error_file_async(format!("{:?} ", why));
                    }
                }
            }
            Err(why) => {
                logging::error_file_async(format!("{:?} ", why));
            }
        }
    }

    Ok(())
}

/// 處理除息日為尚未公布的股票
async fn processing_unannounced_ex_dividend_date(year: i32) -> Result<()> {
    //除息日 尚未公布
    let dividends =
        dividend::Dividend::fetch_unpublished_dividend_date_or_payable_date_for_specified_year(
            year,
        )
        .await?;

    logging::info_file_async(format!(
        "本次除息日與發放日的採集需收集 {} 家",
        dividends.len()
    ));

    let tasks: Vec<_> = dividends
        .into_iter()
        .map(|d| processing_unannounced_ex_dividend_date_from_yahoo(d, year))
        .collect();
    let results = futures::future::join_all(tasks).await;

    for result in results {
        if let Err(why) = result {
            logging::error_file_async(format!(
                "Failed to fetch_dividend_from_yahoo because {:?}",
                why
            ));
        }
    }
    Ok(())
}

/// 從雅虎取得除息日的資料
async fn processing_unannounced_ex_dividend_date_from_yahoo(
    mut entity: dividend::Dividend,
    year: i32,
) -> Result<()> {
    let strategy = ExponentialBackoff::from_millis(100)
        .map(jitter) // add jitter to delays
        .take(5); // limit to 5 retries
    let retry_future = Retry::spawn(strategy, || yahoo::dividend::visit(&entity.security_code));
    let yahoo = match retry_future.await {
        Ok(yahoo_dividend) => yahoo_dividend,
        Err(why) => {
            return Err(anyhow!("{}", why));
        }
    };

    // 取得今年度的股利數據
    if let Some(yahoo_dividend_details) = yahoo.dividend.get(&year) {
        let yahoo_dividend_detail = yahoo_dividend_details.iter().find(|detail| {
            detail.year_of_dividend == entity.year_of_dividend
                && detail.quarter == entity.quarter
                && (detail.ex_dividend_date1 != entity.ex_dividend_date1
                    || detail.ex_dividend_date2 != entity.ex_dividend_date2
                    || detail.payable_date1 != entity.payable_date1
                    || detail.payable_date2 != entity.payable_date2)
        });

        if let Some(yahoo_dividend_detail) = yahoo_dividend_detail {
            entity.ex_dividend_date1 = yahoo_dividend_detail.ex_dividend_date1.to_string();
            entity.ex_dividend_date2 = yahoo_dividend_detail.ex_dividend_date2.to_string();
            entity.payable_date1 = yahoo_dividend_detail.payable_date1.to_string();
            entity.payable_date2 = yahoo_dividend_detail.payable_date2.to_string();

            if let Err(why) = entity.update_dividend_date().await {
                return Err(anyhow!("{}", why));
            }

            logging::info_file_async(format!(
                "dividend update_dividend_date executed successfully. \r\n{:?}",
                entity
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::cache::SHARE;

    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_processing_with_unannounced_ex_dividend_dates() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 processing_with_unannounced_ex_dividend_dates".to_string());

        match processing_unannounced_ex_dividend_date(2023).await {
            Ok(_) => {
                logging::debug_file_async(
                    "processing_with_unannounced_ex_dividend_dates executed successfully."
                        .to_string(),
                );
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to processing_with_unannounced_ex_dividend_dates because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 processing_with_unannounced_ex_dividend_dates".to_string());
    }

    #[tokio::test]
    #[ignore]
    async fn test_processing_without_or_multiple() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        match processing_no_or_multiple(2024).await {
            Ok(_) => {
                logging::debug_file_async(
                    "processing_without_or_multiple executed successfully.".to_string(),
                );
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to processing_without_or_multiple because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 processing_without_or_multiple".to_string());
    }

    #[tokio::test]

    async fn test_process_stock_dividends() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 process_stock_dividends".to_string());
        let year = 2024;
        let multiple_dividends = dividend::Dividend::fetch_multiple_dividends_for_year(year)
            .await
            .unwrap();
        let mut multiple_dividend_cache = HashSet::new();
        for dividend in multiple_dividends {
            let key = dividend.key();
            multiple_dividend_cache.insert(key);
        }

        match process_stock_dividends(year, "2454", &multiple_dividend_cache).await {
            Ok(_) => {
                logging::debug_file_async(
                    "process_stock_dividends executed successfully.".to_string(),
                );
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to process_stock_dividends because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 process_stock_dividends".to_string());
    }
}
