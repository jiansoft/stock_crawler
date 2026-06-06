use crate::{
    core::logging, core::util::datetime::Weekend, domain::registry::repository::StockRepository,
    infra::crawler::twse, infra::database::repository::stock::PgStockRepository,
};
use anyhow::Result;
use chrono::Local;
use scopeguard::defer;

/// 更新資料庫中終止上市的公司
pub async fn execute() -> Result<()> {
    if Local::now().is_weekend() {
        return Ok(());
    }
    logging::info_file_async("更新下市的股票開始");
    defer! {
       logging::info_file_async("更新下市的股票結束");
    }
    let delisted = twse::suspend_listing::visit().await?;
    let mut items_to_update = Vec::new();
    let repo = PgStockRepository::new();

    for company in delisted {
        if let Some(stock) = repo.find_by_symbol(&company.stock_symbol).await? {
            if stock.suspend_listing() {
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
            another.update_suspension(true);
            items_to_update.push(another);
        }
    }

    for stock in items_to_update {
        if let Err(why) = repo.save(&stock).await {
            logging::error_file_async(format!(
                "Failed to update_suspend_listing because {:?}",
                why
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::infra::cache::SHARE;

    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    #[tokio::test]
    #[ignore]
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
