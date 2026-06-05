use anyhow::Result;
use sqlx::postgres::PgQueryResult;

use crate::{infra::cache::SHARE, infra::database::table::stock::extension};

/// 更新興櫃股票的每股淨值
pub mod emerging;
/// 將每股淨值為零的股票嚐試從yahoo取得數據後更新
pub mod zero_value;

/// 更新興櫃股票的每股淨值，資料庫更新後會更新 SHARE.stocks
pub async fn update(stock: &crate::domain::registry::entity::Stock) -> Result<PgQueryResult> {
    let item = extension::net_asset_value_per_share::SymbolAndNetAssetValuePerShare::from(stock);
    let result = item.update().await?;

    if result.rows_affected() > 0 {
        if let Ok(mut stocks_cache) = SHARE.stocks.write() {
            if let Some(stock_cache) = stocks_cache.get_mut(&stock.symbol().0) {
                stock_cache.update_net_asset_value(stock.net_asset_value_per_share());
            }
        }
    }

    Ok(result)
}
