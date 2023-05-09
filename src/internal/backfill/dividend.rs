use crate::internal::crawler::yahoo;
use crate::internal::{
    crawler::goodinfo,
    database::model::{self, dividend},
    logging,
};
use anyhow::*;
use chrono::{Datelike, Local};
use core::result::Result::Ok;
use std::{thread, time::Duration};

/// 更新股利發送數據
/// 資料庫內尚未有年度配息數據的股票取出後向第三方查詢後更新回資料庫
pub async fn execute() -> Result<()> {
    //尚未有股利或多次配息
    let now = Local::now();
    let year = now.year();
    let without_or_multiple = process_without_or_multiple(year);
    let yahoo = process_yahoo(year);
    tokio::try_join!(without_or_multiple, yahoo)?;
    Ok(())
}

/// Asynchronously processes the stocks that have no dividends or have issued multiple dividends.
///
/// This function fetches a list of stock symbols that either have no dividends or have issued multiple dividends
/// in the current year. It then visits the dividend information of each stock symbol using the `goodinfo::dividend::visit`
/// function. For each dividend found, if the dividend's year is not the current year, it skips to the next dividend.
/// Otherwise, it converts the dividend into a `model::dividend::Entity` and tries to upsert it.
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
async fn process_without_or_multiple(year: i32) -> Result<()> {
    //尚未有股利或多次配息
    let stock_symbols = dividend::fetch_without_or_multiple(year).await?;
    for stock_symbol in stock_symbols {
        let dividends = goodinfo::dividend::visit(&stock_symbol).await?;
        for dividend in dividends {
            if dividend.year != year {
                continue;
            }

            let entity = model::dividend::Entity::from(dividend);
            match entity.upsert().await {
                Ok(_) => {
                    logging::info_file_async(format!(
                        "dividend upsert executed successfully. \r\n{:#?}",
                        entity
                    ));
                }
                Err(why) => {
                    logging::error_file_async(format!("Failed to upsert because {:?} ", why));
                }
            }
        }

        thread::sleep(Duration::from_secs(6));
    }

    Ok(())
}

async fn process_yahoo(year: i32) -> Result<()> {
    //除息日 尚未公布
    let dividends = dividend::fetch_unannounced_date(year).await?;
    for mut dividend in dividends {
        let yahoo_dividend = match yahoo::dividend::visit(&dividend.security_code).await {
            Ok(yahoo_dividend) => yahoo_dividend,
            Err(why) => {
                logging::error_file_async(format!(
                    "Failed to yahoo::dividend::visit because {:?} ",
                    why
                ));
                continue;
            }
        };

        if let Some(yahoo_dividend_details) = yahoo_dividend.dividend.get(&year) {
            let yahoo_dividend_detail = yahoo_dividend_details.iter().find(|detail| {
                detail.year_of_dividend == dividend.year_of_dividend
                    && detail.quarter == dividend.quarter
                    && (detail.ex_dividend_date1 != dividend.ex_dividend_date1
                        || detail.ex_dividend_date2 != dividend.ex_dividend_date2)
            });

            if let Some(yahoo_dividend_detail) = yahoo_dividend_detail {
                dividend.ex_dividend_date1 = yahoo_dividend_detail.ex_dividend_date1.to_string();
                dividend.ex_dividend_date2 = yahoo_dividend_detail.ex_dividend_date2.to_string();
                dividend.payable_date1 = yahoo_dividend_detail.payable_date1.to_string();
                dividend.payable_date2 = yahoo_dividend_detail.payable_date2.to_string();

                if let Err(why) = dividend.update_dividend_date().await {
                    logging::error_file_async(format!(
                        "Failed to update_dividend_date because {:?} ",
                        why
                    ));
                } else {
                    logging::info_file_async(format!(
                        "dividend update_dividend_date executed successfully. \r\n{:#?}",
                        dividend
                    ));
                }
            }
        }
        /*if let Some(yahoo_dividend_details) = yahoo_dividend.dividend.get(&year) {
            for yahoo_dividend_detail in yahoo_dividend_details {
                if yahoo_dividend_detail.year_of_dividend != dividend.year_of_dividend
                    || yahoo_dividend_detail.quarter != dividend.quarter
                {
                    continue;
                }
                dividend.ex_dividend_date1 = yahoo_dividend_detail.ex_dividend_date1.to_string();
                dividend.ex_dividend_date2 = yahoo_dividend_detail.ex_dividend_date2.to_string();
                dividend.payable_date1 = yahoo_dividend_detail.payable_date1.to_string();
                dividend.payable_date2 = yahoo_dividend_detail.payable_date2.to_string();
                match dividend.update_dividend_date().await {
                    Ok(_) => {
                        logging::info_file_async(format!(
                            "dividend update_dividend_date executed successfully. \r\n{:#?}",
                            dividend
                        ));
                    }
                    Err(why) => {
                        logging::error_file_async(format!(
                            "Failed to update_dividend_date because {:?} ",
                            why
                        ));
                    }
                }
            }
        }*/
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::internal::cache::SHARE;
    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    #[tokio::test]
    async fn test_execute() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 execute".to_string());

        match process_yahoo(2023).await {
            Ok(_) => {
                logging::debug_file_async("execute executed successfully.".to_string());
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to execute because {:?}", why));
            }
        }

        logging::debug_file_async("結束 execute".to_string());
    }
}
