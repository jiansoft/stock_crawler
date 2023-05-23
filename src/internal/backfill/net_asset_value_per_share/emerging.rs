use crate::internal::{
    backfill::net_asset_value_per_share::update, cache::SHARE, crawler::tpex, database::model,
    logging, util::datetime::Weekend,
};
use anyhow::*;
use chrono::Local;
use core::result::Result::Ok;

/// 更新興櫃股票的每股淨值
pub async fn execute() -> Result<()> {
    if Local::now().is_weekend() {
        return Ok(());
    }

    let result = tpex::net_asset_value_per_share::visit().await?;

    for item in result {
        let stock = match SHARE.stocks.read() {
            Ok(stocks_cache) => {
                if let Some(stock_db) = stocks_cache.get(item.stock_symbol.as_str()) {
                    if stock_db.net_asset_value_per_share == item.net_asset_value_per_share {
                        continue;
                    }
                }
                model::stock::Entity::from(item)
            }
            Err(_) => continue,
        };

        match update(&stock).await {
            Ok(_) => {
                logging::info_file_async(format!(
                    "update_net_asset_value_per_share executed successfully. \r\n{:#?}",
                    stock
                ));
            }
            Err(why) => {
                logging::error_file_async(format!(
                    "Failed to update_net_asset_value_per_share because {:?}",
                    why
                ));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::logging;

    #[tokio::test]
    async fn test_execute() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 execute".to_string());

        match execute().await {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!("Failed to execute because {:?}", why));
            }
        }

        logging::debug_file_async("結束 execute".to_string());
    }
}
