use anyhow::*;
use sqlx::{postgres::PgQueryResult, FromRow};

use crate::internal::database::{self, table::stock};

/// 更新股票的下市狀態
#[derive(FromRow, Debug)]
pub struct SymbolAndSuspendListing {
    pub stock_symbol: String,
    pub suspend_listing: bool,
}

//let entity: SymbolAndSuspendListing = fs.into(); // 或者 let entity = SymbolAndSuspendListing::from(fs);
impl From<&stock::Stock> for SymbolAndSuspendListing {
    fn from(stock: &stock::Stock) -> Self {
        SymbolAndSuspendListing::new(stock.stock_symbol.clone(), stock.suspend_listing)
    }
}

/// 股號和每股淨值
impl SymbolAndSuspendListing {
    pub fn new(stock_symbol: String, suspend_listing: bool) -> Self {
        SymbolAndSuspendListing {
            stock_symbol,
            suspend_listing,
        }
    }

    /// 更新個股的下市狀態
    pub async fn update(&self) -> Result<PgQueryResult> {
        let sql = r#"
update
    stocks
set
    "SuspendListing" = $2
where
    stock_symbol = $1;
"#;
        Ok(sqlx::query(sql)
            .bind(&self.stock_symbol)
            .bind(self.suspend_listing)
            .execute(database::get_connection())
            .await?)
    }
}
