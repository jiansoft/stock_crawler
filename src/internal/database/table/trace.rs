use anyhow::{Context, Result};
use rust_decimal::Decimal;
use sqlx::{postgres::PgRow, QueryBuilder, Row};

use crate::{internal::database, util::map::Keyable};

#[derive(sqlx::Type, sqlx::FromRow, Debug)]
pub struct Trace {
    pub stock_symbol: String,
    pub floor: Decimal,
    pub ceiling: Decimal,
}

impl Trace {
    pub fn new(stock_symbol: String, floor: Decimal, ceiling: Decimal) -> Self {
        Trace {
            stock_symbol,
            floor,
            ceiling,
        }
    }

    /// 從資料表中取得進行追踪的股票
    pub async fn fetch() -> Result<Vec<Trace>> {
        QueryBuilder::new("select stock_symbol,floor,ceiling from trace")
            .build()
            .try_map(|row: PgRow| {
                let ceiling = row.try_get("ceiling")?;
                let floor = row.try_get("floor")?;
                let stock_symbol = row.try_get("stock_symbol")?;
                Ok(Trace::new(stock_symbol, floor, ceiling))
            })
            .fetch_all(database::get_connection())
            .await
            .context("Failed to Trace::fetch() from database".to_string())
    }
}

impl Keyable for Trace {
    fn key(&self) -> String {
        format!(
            "Trace:{}-{}-{}",
            &self.stock_symbol, self.floor, self.ceiling
        )
    }
}

impl Clone for Trace {
    fn clone(&self) -> Self {
        Self {
            stock_symbol: self.stock_symbol.clone(),
            floor: self.floor,
            ceiling: self.ceiling,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        internal::{database::table::trace},
        logging
    };

    #[tokio::test]
    #[ignore]
    async fn test_fetch_list() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_list".to_string());

        let r = trace::Trace::fetch().await;
        if let Ok(result) = r {
            dbg!(&result);
            logging::debug_file_async(format!("{:#?}", result));
        } else if let Err(err) = r {
            logging::debug_file_async(format!("{:#?} ", err));
        }
        logging::debug_file_async("結束 fetch_list".to_string());
    }
}
