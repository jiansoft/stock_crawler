use crate::{
    app::backfill::net_asset_value_per_share::update, core::logging,
    infra::crawler::yahoo::profile, infra::database::table,
};
use anyhow::Result;

/// 將未下市每股淨值為零的股票試著到 yahoo 抓取數據後更新回 stocks表
pub async fn execute() -> Result<()> {
    let stocks = table::stock::fetch_net_asset_value_per_share_is_zero().await?;
    let domain_stocks: Vec<crate::domain::registry::entity::Stock> =
        stocks.into_iter().map(Into::into).collect();

    for mut stock in domain_stocks {
        if stock.symbol().is_preference() || stock.symbol().is_tdr() {
            continue;
        }

        let profile_skip_cache_key = profile::no_valid_data_cache_key(&stock.symbol().0);
        if crate::infra::nosql::redis::CLIENT
            .get_bool(&profile_skip_cache_key)
            .await?
        {
            continue;
        }

        let yahoo_profile = match profile::visit(&stock.symbol().0).await {
            Ok(stock_profile) => stock_profile,
            Err(why) => {
                if profile::is_no_valid_data_error(&why) {
                    if let Err(cache_err) = crate::infra::nosql::redis::CLIENT
                        .set(
                            &profile_skip_cache_key,
                            true,
                            profile::NO_VALID_DATA_CACHE_TTL_SECONDS,
                        )
                        .await
                    {
                        logging::error_file_async(format!(
                            "Failed to cache profile::visit no-valid-data skip for {} because {:?}",
                            stock.symbol().0,
                            cache_err
                        ));
                    }
                    logging::warn_file_async(format!(
                        "Skip profile::visit for {} because {}",
                        stock.symbol().0,
                        why
                    ));
                } else {
                    logging::error_file_async(format!(
                        "Failed to profile::visit for {} because {}",
                        stock.symbol().0,
                        why
                    ));
                }
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

        stock.update_net_asset_value(yahoo_profile.net_asset_value_per_share);

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
    use crate::core::logging;

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
