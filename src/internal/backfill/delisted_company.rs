use crate::{
    internal::cache_share::CACHE_SHARE, internal::crawler::twse, internal::util::datetime::Weekend,
    logging,
};

use anyhow::*;
use chrono::Local;
use core::result::Result::Ok;

/// 更新資料庫中終止上市的公司
pub async fn execute() -> Result<()> {
    if Local::now().is_weekend() {
        return Ok(());
    }

    let delisted_list = twse::suspend_listing::visit().await;

    if let Some(delisted) = delisted_list {
        let mut items_to_update = Vec::new();

        match CACHE_SHARE.stocks.read() {
            Ok(stocks) => {
                for company in delisted {
                    if let Some(stock) = stocks.get(company.stock_symbol.as_str()) {
                        if stock.suspend_listing {
                            //println!("已下市{:?}",stock);
                            continue;
                        }

                        let year = match company.delisting_date[..3].parse::<i32>() {
                            Ok(_year) => _year,
                            Err(why) => {
                                logging::error_file_async(format!(
                                    "轉換資料日期發生錯誤. because {:?}",
                                    why
                                ));
                                continue;
                            }
                        };

                        if year < 110 {
                            continue;
                        }

                        let mut another = stock.clone();
                        another.suspend_listing = true;
                        items_to_update.push(another);
                    }
                }
            }
            Err(why) => {
                logging::error_file_async(format!("Failed to read stocks cache because {:?}", why));
            }
        }

        for item in items_to_update {
            if let Err(why) = item.update_suspend_listing().await {
                logging::error_file_async(format!(
                    "Failed to update_suspend_listing because {:?}",
                    why
                ));
            } else if let Ok(mut stocks_cache) = CACHE_SHARE.stocks.write() {
                if let Some(stock) = stocks_cache.get_mut(&item.stock_symbol) {
                    stock.suspend_listing = true;
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::internal::cache_share::CACHE_SHARE;
    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    #[tokio::test]
    async fn test_execute() {
        dotenv::dotenv().ok();
        CACHE_SHARE.load().await;
        logging::debug_file_async("開始 execute".to_string());

        match execute().await {
            Ok(_) => {
                logging::debug_file_async("execute executed successfully.".to_string());
            }
            Err(why) => {
                logging::error_file_async(format!("Failed to execute because {:?}", why));
            }
        }

        logging::debug_file_async("結束 execute".to_string());
    }
}
