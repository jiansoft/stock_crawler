//! 收盤後寫入每日風險指標（`daily_risk_metric`：近 240 筆日線的年化波動度與對加權指數的 beta）。

use anyhow::{Context, Result};
use chrono::NaiveDate;

use crate::infra::database::repository::risk_metric::PgRiskMetricRepository;

/// 重算並寫入 `date` 的風險指標，回傳寫入列數；不是交易日時為 0。
///
/// # Errors
///
/// 查詢或寫入失敗時回傳錯誤。
pub async fn calculate_risk_metrics(date: NaiveDate) -> Result<u64> {
    calculate_risk_metrics_between(date, date).await
}

/// 重算 `from`～`to`（含）每個交易日的風險指標（手動回補用），回傳寫入列數。
///
/// # Errors
///
/// 查詢或寫入失敗時回傳錯誤。
pub async fn calculate_risk_metrics_between(from: NaiveDate, to: NaiveDate) -> Result<u64> {
    PgRiskMetricRepository::new()
        .replace_range(from, to)
        .await
        .with_context(|| format!("寫入 {from}～{to} 的風險指標失敗"))
}

#[cfg(test)]
mod tests;
