use crate::{app::backfill::acl::NetAssetValueAclMapper, app::backfill::net_asset_value_per_share::update, infra::crawler::yahoo::profile};
use anyhow::Result;

/// 將未下市每股淨值為零的股票試著到 yahoo 抓取數據後更新回 stocks表
pub async fn execute() -> Result<()> {
    let stock_repo = crate::infra::database::repository::stock::PgStockRepository::new();
    use crate::domain::registry::repository::StockRepository;
    let domain_stocks = stock_repo.fetch_net_asset_value_per_share_is_zero().await?;

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
                        tracing::error!("Failed to cache profile::visit no-valid-data skip for {} because {:?}",
                            stock.symbol().0,
                            cache_err);
                    }
                    tracing::warn!("Skip profile::visit for {} because {}",
                        stock.symbol().0,
                        why);
                } else {
                    tracing::error!("Failed to profile::visit for {} because {}",
                        stock.symbol().0,
                        why);
                }
                continue;
            }
        };

        let cmd =
            NetAssetValueAclMapper::from_yahoo_profile(stock.symbol().0.clone(), &yahoo_profile);

        if cmd.net_asset_value_per_share.is_zero() {
            tracing::info!("the stock's net_asset_value_per_share is zero still. \r\n{:#?}",
                yahoo_profile);
            continue;
        }

        stock.update_net_asset_value(cmd.net_asset_value_per_share);

        if let Err(why) = update(&stock).await {
            tracing::error!("Failed to update_net_asset_value_per_share because {:?}",
                why);
            continue;
        }

        tracing::info!("zero update_net_asset_value_per_share executed successfully. \r\n{:#?}",
            stock);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenv::dotenv().ok();
        tracing::debug!("開始 execute");

        match execute().await {
            Ok(_) => {}
            Err(why) => {
                tracing::debug!("Failed to execute because {:?}", why);
            }
        }

        tracing::debug!("結束 execute");
    }
}
