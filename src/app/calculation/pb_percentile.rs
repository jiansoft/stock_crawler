//! 收盤後寫入每日股價淨值比歷史分位（`daily_pb_percentile`：當天股價淨值比在自身近 5 年的百分位）。

use anyhow::{Context, Result};
use chrono::NaiveDate;

use crate::infra::database::repository::pb_percentile::PgPbPercentileRepository;

/// 重算並寫入 `date` 的股價淨值比分位，回傳寫入列數；不是交易日時為 0。
///
/// # Errors
///
/// 查詢或寫入失敗時回傳錯誤。
pub async fn calculate_pb_percentiles(date: NaiveDate) -> Result<u64> {
    calculate_pb_percentiles_between(date, date).await
}

/// 重算 `from`～`to`（含）每個交易日的股價淨值比分位（手動回補用），回傳寫入列數。
///
/// # Errors
///
/// 查詢或寫入失敗時回傳錯誤。
pub async fn calculate_pb_percentiles_between(from: NaiveDate, to: NaiveDate) -> Result<u64> {
    PgPbPercentileRepository::new()
        .replace_range(from, to)
        .await
        .with_context(|| format!("寫入 {from}～{to} 的股價淨值比分位失敗"))
}

#[cfg(test)]
mod tests;
