use crate::{
    internal::backfill::net_asset_value_per_share::update, internal::crawler::yahoo::profile,
    internal::database::model, logging,
};
use anyhow::*;
use core::result::Result::Ok;
use rust_decimal::Decimal;

/// 將未下市每股淨值為零的股票試著到 yahoo 抓取數據後更新回 stocks表
pub async fn execute() -> Result<()> {
    let stocks = model::stock::fetch_net_asset_value_per_share_is_zero().await?;
    for mut stock in stocks {
        if stock.is_preference_shares() || stock.is_tdr() {
            continue;
        }

        match profile::visit(&stock.stock_symbol).await {
            Ok(stock_profile) => {
                if stock_profile.net_asset_value_per_share == Decimal::ZERO {
                    logging::info_file_async(format!(
                        "the stock's net_asset_value_per_share is zero still. \r\n{:#?}",
                        stock_profile
                    ));

                    continue;
                }

                stock.net_asset_value_per_share = stock_profile.net_asset_value_per_share;

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
            Err(why) => {
                logging::error_file_async(format!("Failed to profile::visit because {:?}", why));
            }
        };
    }

    Ok(())
}

/*async fn process_stock(stock: &model::stock::Entity) -> Result<()> {
    let stock_profile = profile::visit(&stock.stock_symbol).await.map_err(|why| {
        logging::error_file_async(format!(
            "Failed to net_asset_value_per_share::visit because {:?}",
            why
        ));
        why
    })?;

    let mut e = model::stock::Entity::new();
    e.stock_symbol = stock_profile.security_code;
    e.net_asset_value_per_share = stock_profile.net_asset_value_per_share;

    match e.update_net_asset_value_per_share().await {
        Ok(_) => Ok(()),
        Err(why) => Err(anyhow!(
            "Failed to net_asset_value_per_share::visit because {:?}",
            why
        )),
    }
}

pub async fn execute() -> Result<()> {
    let stocks = model::stock::fetch_net_asset_value_per_share_is_zero().await?;
    let futures = stocks.iter().map(process_stock);
    let results = futures::future::join_all(futures).await;

    // Log any errors that occurred during processing.
    for error in results.into_iter().filter_map(Result::err) {
        logging::error_file_async(format!("Failed to process stock because {:?}", error));
    }

    Ok(())
}
*/

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging;

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
