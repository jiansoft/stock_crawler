use anyhow::{anyhow, Context, Result};
use rust_decimal::Decimal;
use sqlx::{postgres::PgQueryResult, FromRow};

use crate::{crawler::taifex::stock_weight::StockWeight, database};

/// 更新股票的權重
#[derive(FromRow, Debug, Clone)]
pub struct SymbolAndWeight {
    pub stock_symbol: String,
    //權植佔比
    pub weight: Decimal,
}

//let entity: Entity = fs.into(); // 或者 let entity = Entity::from(fs);
impl From<StockWeight> for SymbolAndWeight {
    fn from(stock_weight: StockWeight) -> Self {
        SymbolAndWeight::new(stock_weight.stock_symbol, stock_weight.weight)
    }
}

// 新增一個方法來將 StockWeight 轉換成 SymbolAndWeight
pub fn from(weights: Vec<StockWeight>) -> Vec<SymbolAndWeight> {
    weights.into_iter().map(SymbolAndWeight::from).collect()
}

impl SymbolAndWeight {
    pub fn new(stock_symbol: String, weight: Decimal) -> Self {
        SymbolAndWeight {
            stock_symbol,
            weight,
        }
    }

    /// 更新個股的權值佔比
    pub async fn update(&self) -> Result<PgQueryResult> {
        let sql = r#"
UPDATE
    stocks
SET
    weight = $2
WHERE
    stock_symbol = $1;
"#;
        sqlx::query(sql)
            .bind(&self.stock_symbol)
            .bind(self.weight)
            .execute(database::get_connection())
            .await
            .context("Failed to update weight from database")
    }

    /// 個股的權值佔比歸零
    pub async fn zeroed_out() -> Result<PgQueryResult> {
        let sql = "UPDATE stocks SET weight = 0";
        let mut tx = database::get_tx().await?;
        let result = match sqlx::query(sql).execute(&mut *tx).await {
            Ok(result) => result,
            Err(why) => {
                tx.rollback().await?;
                return Err(anyhow!("Failed to zeroed_out because {:?}", why));
            }
        };

        tx.commit().await?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use core::result::Result::Ok;

    use rust_decimal_macros::dec;

    use crate::{database::table::stock::Stock, logging};

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_update() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 update".to_string());
        let stock_weight = StockWeight {
            rank: 0,
            stock_symbol: "2330".to_string(),
            weight: dec!(28.3278),
        };

        let sw = SymbolAndWeight::from(stock_weight);
        match sw.update().await {
            Ok(_qr) => {
                let sql = r#"
SELECT
    stock_symbol,
    "Name" AS name,
    "SuspendListing" AS suspend_listing,
    "CreateTime" AS create_time,
    net_asset_value_per_share,
    weight,
    stock_exchange_market_id,
    stock_industry_id
FROM stocks
WHERE stock_symbol = $1;
    "#;

                let stock = sqlx::query_as::<_, Stock>(sql)
                    .bind(&sw.stock_symbol)
                    .fetch_one(database::get_connection())
                    .await;
                match stock {
                    Ok(s) => {
                        assert_eq!(s.weight, sw.weight);

                        logging::debug_file_async(format!("stock:{:?}", s));
                        dbg!(s);
                    }
                    Err(why) => {
                        logging::debug_file_async(format!(
                            "Failed to fetch stock because: {:?}",
                            why
                        ));
                    }
                }
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to update stock weight because: {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 update".to_string());
    }

    #[tokio::test]
    #[ignore]
    async fn test_zeroed_out() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 zeroed_out".to_string());

        match SymbolAndWeight::zeroed_out().await {
            Ok(_qr) => {}
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to stock weight zeroed_out because: {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 zeroed_out".to_string());
    }
}
