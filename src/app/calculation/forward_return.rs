//! 收盤後回填未來報酬。
//!
//! 交易日 `end` 收盤時，對每個天期 N（[`FORWARD_HORIZONS`]）找出 `end` 往前第 N 個交易日，
//! 把它到 `end` 的還原報酬寫入 `daily_forward_return`。每天各天期只回填一個基準日，
//! 所以只要每個交易日都跑過一次，所有基準日的各天期都會補齊；重跑會覆寫同樣的值。

use anyhow::{Context, Result};
use chrono::NaiveDate;

use crate::domain::quote::forward_return::{self, FORWARD_HORIZONS, ForwardReturnRepository};
use crate::infra::database::repository::quote::PgQuoteRepository;

/// 回填 `end` 收盤時已滿各天期的基準日，回傳寫入的列數；`end` 不是交易日時不寫入。
///
/// # Errors
///
/// 查詢交易日或寫入失敗時回傳錯誤。
pub async fn calculate_forward_returns(end: NaiveDate) -> Result<u64> {
    run(&PgQuoteRepository::new(), end).await
}

/// [`calculate_forward_returns`] 的本體，倉儲可替換以便測試。
pub(crate) async fn run(repo: &impl ForwardReturnRepository, end: NaiveDate) -> Result<u64> {
    let longest = FORWARD_HORIZONS.iter().copied().max().unwrap_or_default();
    let dates = repo
        .fetch_trading_dates(end, longest + 1)
        .await
        .context("查詢交易日失敗")?;
    let mut written = 0;
    for horizon in FORWARD_HORIZONS {
        let Some(base) = forward_return::base_date(end, horizon, &dates) else {
            continue;
        };
        written += repo
            .upsert_forward_returns(base, end, horizon)
            .await
            .with_context(|| format!("寫入 {base} 的 {horizon} 日報酬失敗"))?;
    }
    Ok(written)
}

#[cfg(test)]
mod tests;
