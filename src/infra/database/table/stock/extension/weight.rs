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
mod tests;
