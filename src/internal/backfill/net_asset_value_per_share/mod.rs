/// 更新興櫃股票的每股淨值
pub mod emerging;
/// 將每股淨值為零的股票嚐試從yahoo取得數據後更新
pub mod zero_value;

use crate::{internal::{
    cache::SHARE,
    database::table
}};
use anyhow::*;
use core::result::Result::Ok;
use sqlx::postgres::PgQueryResult;

/// 更新興櫃股票的每股淨值，資料庫更新後會更新 SHARE.stocks
pub async fn update(stock: &table::stock::Stock) -> Result<PgQueryResult> {
    let result = stock.update_net_asset_value_per_share().await?;

    if result.rows_affected() > 0 {
        if let Ok(mut stocks_cache) = SHARE.stocks.write() {
            if let Some(stock_cache) = stocks_cache.get_mut(&stock.stock_symbol) {
                stock_cache.net_asset_value_per_share = stock.net_asset_value_per_share;
            }
        }
    }

    Ok(result)
}
