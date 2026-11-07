//! 證券主檔的權值占比：全部歸零與逐檔更新。

use anyhow::{Context, Result};
use rust_decimal::Decimal;

use crate::infra::database;

/// 將所有有效證券的權值占比重置歸零。
pub(super) async fn zeroed_out_weights() -> Result<()> {
    let sql = "UPDATE stocks SET weight = 0";
    let mut tx = database::get_tx().await?;
    // 失敗時 `tx` 隨 `?` 提前返回被丟棄，sqlx 會自動 rollback。
    sqlx::query(sql)
        .execute(&mut *tx)
        .await
        .context("Failed to zeroed_out_weights in PgStockRepository")?;
    tx.commit().await?;
    Ok(())
}

/// 更新指定證券代號的權值占比。
pub(super) async fn update_weight(stock_symbol: &str, weight: Decimal) -> Result<()> {
    let sql = r#"
            UPDATE stocks
            SET weight = $2
            WHERE stock_symbol = $1;
        "#;
    sqlx::query(sql)
        .bind(stock_symbol)
        .bind(weight)
        .execute(database::get_connection())
        .await
        .context("Failed to update_weight in PgStockRepository")?;
    Ok(())
}
