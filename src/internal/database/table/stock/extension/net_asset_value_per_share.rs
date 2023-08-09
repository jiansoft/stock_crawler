use anyhow::*;
use rust_decimal::Decimal;
use sqlx::{postgres::PgQueryResult, FromRow};

use crate::internal::database::{self, table::stock};

/// 更新股票的每股淨值
#[derive(FromRow, Debug)]
pub struct SymbolAndNetAssetValuePerShare {
    pub stock_symbol: String,
    //每股淨值
    pub net_asset_value_per_share: Decimal,
}

//let entity: Entity = fs.into(); // 或者 let entity = Entity::from(fs);
impl From<&stock::Stock> for SymbolAndNetAssetValuePerShare {
    fn from(stock: &stock::Stock) -> Self {
        SymbolAndNetAssetValuePerShare::new(
            stock.stock_symbol.clone(),
            stock.net_asset_value_per_share,
        )
    }
}

/// 股號和每股淨值
impl SymbolAndNetAssetValuePerShare {
    pub fn new(stock_symbol: String, net_asset_value_per_share: Decimal) -> Self {
        SymbolAndNetAssetValuePerShare {
            stock_symbol,
            net_asset_value_per_share,
        }
    }

    /// 更新個股的每股淨值
    pub async fn update(&self) -> Result<PgQueryResult> {
        let sql = r#"
update
    stocks
set
    net_asset_value_per_share = $2
where
    stock_symbol = $1;
"#;
        sqlx::query(sql)
            .bind(&self.stock_symbol)
            .bind(self.net_asset_value_per_share)
            .execute(database::get_connection())
            .await
            .context("Failed to update net_asset_value_per_share from database")
    }
}
