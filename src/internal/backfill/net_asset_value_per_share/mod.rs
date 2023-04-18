/// 更新興櫃股票的每股淨值
pub mod emerging;
/// 將每股淨值為零的股票嚐試從yahoo取得數據後更新
pub mod zero_value;

use crate::{internal::cache::SHARE, internal::database::model};
use anyhow::*;
use core::result::Result::Ok;

async fn update(stock: &model::stock::Entity) -> Result<()> {
    let c = stock.update_net_asset_value_per_share().await?;

    if c.rows_affected() > 0 {
        if let Ok(mut stocks_cache) = SHARE.stocks.write() {
            if let Some(stock_in_cache) = stocks_cache.get_mut(&stock.stock_symbol) {
                stock_in_cache.net_asset_value_per_share = stock.net_asset_value_per_share;
            }
        }
    }

    Ok(())
}
