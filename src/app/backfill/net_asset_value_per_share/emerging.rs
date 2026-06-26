use crate::{
    app::backfill::acl::NetAssetValueAclMapper, app::backfill::net_asset_value_per_share::update,
    core::util::datetime::Weekend, domain::registry::repository::StockRepository,
    infra::crawler::tpex, infra::database::repository::stock::PgStockRepository,
};
use anyhow::Result;
use chrono::Local;
use scopeguard::defer;

/// 更新興櫃股票的每股淨值
pub async fn execute() -> Result<()> {
    if Local::now().is_weekend() {
        return Ok(());
    }

    tracing::info!("更新興櫃股票的每股淨值開始");
    defer! {
      tracing::info!("更新興櫃股票的每股淨值結束");
    }

    let result = tpex::net_asset_value_per_share::visit().await?;
    let repo = PgStockRepository::new();

    for item in result {
        let cmd = NetAssetValueAclMapper::from_emerging(&item);
        let stock_cache = repo.find_by_symbol(&cmd.symbol).await?;
        let stock = match stock_cache {
            None => continue,
            Some(stock_cache) => {
                if stock_cache.net_asset_value_per_share() == cmd.net_asset_value_per_share {
                    continue;
                }
                let mut s = stock_cache.clone();
                s.update_net_asset_value(cmd.net_asset_value_per_share);
                s
            }
        };

        match update(&stock).await {
            Ok(_) => {
                tracing::info!(
                    "emerging update_net_asset_value_per_share executed successfully. \r\n{:#?}",
                    stock
                );
            }
            Err(why) => {
                tracing::error!("{:?}", why);
            }
        }
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
