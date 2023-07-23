use core::result::Result::Ok;

use anyhow::*;
use chrono::Local;

use crate::internal::{
    cache::SHARE, crawler::twse, database::table::stock, logging, util::datetime::Weekend,
};

/// 更新資料庫中終止上市的公司
pub async fn execute() -> Result<()> {
    if Local::now().is_weekend() {
        return Ok(());
    }

    let delisted = twse::suspend_listing::visit().await?;
    let mut items_to_update = Vec::new();

    let stocks = match SHARE.stocks.read() {
        Ok(stocks) => stocks.clone(),
        Err(why) => {
            return Err(anyhow!("Failed to read stocks cache because {:?}", why));
        }
    };

    for company in delisted {
        if let Some(stock) = stocks.get(company.stock_symbol.as_str()) {
            if stock.suspend_listing {
                //println!("已下市{:?}",stock);
                continue;
            }

            if company.delisting_date.len() < 3 {
                continue;
            }

            let year = match company.delisting_date[..3].parse::<i32>() {
                Ok(_year) => _year,
                Err(why) => {
                    logging::error_file_async(format!("轉換資料日期發生錯誤. because {:?}", why));
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

    for stock in items_to_update {
        let item = stock::extension::suspend_listing::SymbolAndSuspendListing::from(&stock);
        if let Err(why) = item.update().await {
            logging::error_file_async(format!(
                "Failed to update_suspend_listing because {:?}",
                why
            ));
        } else if let Ok(mut stocks_cache) = SHARE.stocks.write() {
            if let Some(stock) = stocks_cache.get_mut(&item.stock_symbol) {
                stock.suspend_listing = true;
            }
        }
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

        match execute().await {
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
