use crate::{
    internal::crawler::financial_statement::yahoo::net_asset_value_per_share,
    internal::database::model, logging,
};
use anyhow::*;
use core::result::Result::Ok;

/// 將未下市每股淨值為零的股票試著到yahoo 抓取數據後更新回 stocks表
pub async fn execute() -> Result<()> {
    let stocks = model::stock::fetch_net_asset_value_per_share_is_zero().await?;
    for stock in stocks {
        if let Err(why) = net_asset_value_per_share::visit(&stock.stock_symbol).await {
            logging::error_file_async(format!(
                "Failed to net_asset_value_per_share::visit because {:?}",
                why
            ));
        }
    }

    Ok(())
}
