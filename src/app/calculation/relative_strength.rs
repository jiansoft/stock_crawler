//! 收盤後寫入每日相對強度。
//!
//! 回填未來報酬之後，以當天的近 60／120／240 日報酬對上市櫃普通股（不含 ETF、特別股）計分排名，整天替換
//! `daily_relative_strength`（見 [`crate::domain::quote::relative_strength`]）。當天報酬還沒齊
//! （全體計分退回前一個交易日）或不是交易日時不寫入。

use std::collections::HashMap;

use anyhow::{Context, Result};
use chrono::{Days, NaiveDate};

use crate::domain::quote::relative_strength::{
    RelativeStrengthQuery, RelativeStrengthRepository, ScoredUniverseQuery, crossing_signals,
};
use crate::infra::database::repository::quote::PgQuoteRepository;

/// 計算並寫入 `date` 的每日相對強度，回傳寫入列數；當天沒有可計分的資料時回 0。
///
/// # Errors
///
/// 查詢或寫入失敗時回傳錯誤。
pub async fn calculate_relative_strength(date: NaiveDate) -> Result<u64> {
    run(&PgQuoteRepository::new(), date).await
}

/// [`calculate_relative_strength`] 的本體，倉儲可替換以便測試。
///
/// 同時以前一個已寫入的交易日分數比對，寫入當天穿越 80、50 分的技術訊號（見
/// [`crossing_signals`]），供訊號選股與訊號績效使用。
pub(crate) async fn run(
    repo: &(impl ScoredUniverseQuery + RelativeStrengthQuery + RelativeStrengthRepository),
    date: NaiveDate,
) -> Result<u64> {
    let Some((data_date, scored)) = repo
        .fetch_scored_universe(date)
        .await
        .context("計算全體相對強度失敗")?
    else {
        return Ok(0);
    };
    if data_date != date {
        return Ok(0);
    }
    let symbols: Vec<String> = scored
        .iter()
        .map(|stock| stock.row.stock_symbol.clone())
        .collect();
    let previous: HashMap<String, u8> = repo
        .fetch_relative_strength(date - Days::new(1), 1, &symbols)
        .await
        .context("讀取前一個交易日的相對強度失敗")?
        .into_iter()
        .map(|day| (day.stock_symbol, day.rs_score))
        .collect();
    let signals = crossing_signals(date, &previous, &scored);
    repo.replace_relative_strength(date, &scored, &signals)
        .await
        .with_context(|| format!("寫入 {date} 的相對強度失敗"))
}

#[cfg(test)]
mod tests;
