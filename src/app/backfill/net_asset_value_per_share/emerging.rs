use crate::{
    app::backfill::acl::NetAssetValueAclMapper, app::backfill::net_asset_value_per_share::update,
    core::util::datetime::Weekend, domain::registry::repository::StockRepository,
    infra::crawler::tpex, infra::crawler::tpex::net_asset_value_per_share::Emerging,
    infra::database::repository::stock::PgStockRepository,
};
use anyhow::Result;
use chrono::{DateTime, Local};
use scopeguard::defer;

/// 更新興櫃股票的每股淨值
pub async fn execute() -> Result<()> {
    run(Local::now()).await
}

/// 以 `now` 判斷是否為週末（來源週末不更新，直接結束），平日抓取興櫃名冊後寫回有變動的淨值。
async fn run(now: DateTime<Local>) -> Result<()> {
    if now.is_weekend() {
        return Ok(());
    }

    tracing::info!("更新興櫃股票的每股淨值開始");
    defer! {
      tracing::info!("更新興櫃股票的每股淨值結束");
    }

    let result = tpex::net_asset_value_per_share::visit().await?;
    update_changed(&result).await?;

    Ok(())
}

/// 逐筆比對股票主檔的每股淨值，只有數值不同才寫回；主檔沒有的代號略過。
///
/// 回傳實際寫回的筆數；單筆寫入失敗只記錄錯誤、不中斷其餘資料。
///
/// # Errors
///
/// 查詢股票主檔失敗時回傳錯誤。
async fn update_changed(items: &[Emerging]) -> Result<usize> {
    let repo = PgStockRepository::new();
    let mut updated = 0;

    for item in items {
        let cmd = NetAssetValueAclMapper::from_emerging(item);
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
                updated += 1;
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

    Ok(updated)
}

#[cfg(test)]
mod tests;
