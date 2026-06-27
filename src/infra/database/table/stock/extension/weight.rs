#![allow(dead_code)]
use anyhow::{Context, Result, anyhow};
use rust_decimal::Decimal;
use sqlx::{FromRow, postgres::PgQueryResult};

use crate::infra::database;

/// 更新股票的權重
#[derive(FromRow, Debug, Clone)]
pub struct SymbolAndWeight {
    /// 股票代號。
    pub stock_symbol: String,
    /// 權值占比。
    pub weight: Decimal,
}

impl SymbolAndWeight {
    /// 建立「股票代號 + 權重」資料。
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

    use crate::infra::database::table::stock::StockDbRow;

    use super::*;

    #[tokio::test]
    async fn test_update() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 update");
        let sw = SymbolAndWeight::new("2330".to_string(), dec!(28.3278));
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

                let stock = sqlx::query_as::<_, StockDbRow>(sql)
                    .bind(&sw.stock_symbol)
                    .fetch_one(database::get_connection())
                    .await;
                match stock {
                    Ok(s) => {
                        assert_eq!(s.weight, sw.weight);

                        tracing::debug!("stock:{:?}", s);
                        dbg!(s);
                    }
                    Err(why) => {
                        tracing::debug!("Failed to fetch stock because: {:?}", why);
                    }
                }
            }
            Err(why) => {
                tracing::debug!("Failed to update stock weight because: {:?}", why);
            }
        }

        tracing::debug!("結束 update");
    }

    #[tokio::test]
    async fn test_zeroed_out() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 zeroed_out");

        match SymbolAndWeight::zeroed_out().await {
            Ok(_qr) => {}
            Err(why) => {
                tracing::debug!("Failed to stock weight zeroed_out because: {:?}", why);
            }
        }

        tracing::debug!("結束 zeroed_out");
    }
}
