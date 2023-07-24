use core::result::Result::Ok;

use anyhow::*;
use chrono::Local;

use crate::internal::{
    backfill::net_asset_value_per_share::update, crawler::yahoo::profile, database::table, logging,
    util::datetime::Weekend,
};

/// 將未下市每股淨值為零的股票試著到 yahoo 抓取數據後更新回 stocks表
pub async fn execute() -> Result<()> {
    if Local::now().is_weekend() {
        return Ok(());
    }

    let stocks = table::stock::fetch_net_asset_value_per_share_is_zero().await?;
    for mut stock in stocks {
        if stock.is_preference_shares() || stock.is_tdr() {
            continue;
        }

        let yahoo_profile = match profile::visit(&stock.stock_symbol).await {
            Ok(stock_profile) => stock_profile,
            Err(why) => {
                logging::error_file_async(format!("Failed to profile::visit because {:?}", why));
                continue;
            }
        };

        if yahoo_profile.net_asset_value_per_share.is_zero() {
            logging::info_file_async(format!(
                "the stock's net_asset_value_per_share is zero still. \r\n{:#?}",
                yahoo_profile
            ));
            continue;
        }

        stock.net_asset_value_per_share = yahoo_profile.net_asset_value_per_share;

        if let Err(why) = update(&stock).await {
            logging::error_file_async(format!(
                "Failed to update_net_asset_value_per_share because {:?}",
                why
            ));
            continue;
        }

        logging::info_file_async(format!(
            "zero update_net_asset_value_per_share executed successfully. \r\n{:#?}",
            stock
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::internal::logging;

    use super::*;

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
