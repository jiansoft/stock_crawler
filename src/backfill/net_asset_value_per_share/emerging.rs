use anyhow::Result;
use chrono::Local;
use scopeguard::defer;
use crate::{
    backfill::net_asset_value_per_share::update, cache::SHARE, crawler::tpex, database::table,
    logging, util::datetime::Weekend,
};

/// 更新興櫃股票的每股淨值
pub async fn execute() -> Result<()> {
    if Local::now().is_weekend() {
        return Ok(());
    }

    logging::info_file_async("更新興櫃股票的每股淨值開始");
    defer! {
       logging::info_file_async("更新興櫃股票的每股淨值結束");
    }
    
    let result = tpex::net_asset_value_per_share::visit().await?;

    for item in result {
        let stock_cache = SHARE.get_stock(&item.stock_symbol).await;
        let stock = match stock_cache {
            None => continue,
            Some(stock_cache) => {
                if stock_cache.net_asset_value_per_share == item.net_asset_value_per_share {
                    continue;
                }
                table::stock::Stock::from(item)
            }
        };

        /*
        let stock = match SHARE.stocks.read() {
            Ok(stocks_cache) => {
                if let Some(stock_cache) = stocks_cache.get(item.stock_symbol.as_str()) {
                    if stock_cache.net_asset_value_per_share == item.net_asset_value_per_share {
                        continue;
                    }
                }
                table::stock::Stock::from(item)
            }
            Err(_) => continue,
        };*/

        match update(&stock).await {
            Ok(_) => {
                logging::info_file_async(format!(
                    "emerging update_net_asset_value_per_share executed successfully. \r\n{:#?}",
                    stock
                ));
            }
            Err(why) => {
                logging::error_file_async(format!("{:?}", why));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[tokio::test]
    #[ignore]
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
