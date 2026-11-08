//! 0050 停牌期間缺失的基準報酬，以與日頻路徑相同的還原日線補算。

use std::{collections::HashMap, sync::Arc};

use chrono::Duration;

use super::{ETF_BENCHMARK, PeriodRow, stops::OpenRatios};
use crate::domain::quote::technical::PriceBar;

/// 只補已到期且缺報酬的期數；買進參考價最多回看 30 個日曆天，出場必須有當日有效收盤。
/// 次日開盤成交沿用 SQL 進場日，停牌時開收比取 1；這是估算而非可實際成交的價格。
pub(super) fn fill_missing_returns(
    rows: &mut [PeriodRow],
    bars: &HashMap<String, Arc<Vec<PriceBar>>>,
    ratios: Option<&OpenRatios>,
) {
    let Some(series) = bars.get(ETF_BENCHMARK) else {
        return;
    };
    for row in rows
        .iter_mut()
        .filter(|row| !row.pending && row.etf_return.is_none())
    {
        let Some(exit) = row.exit_date else { continue };
        let start = if ratios.is_some() {
            let Some(start) = row.entry_date else {
                continue;
            };
            start
        } else {
            row.date
        };
        if start > exit {
            continue;
        }
        let at = series.partition_point(|bar| bar.date <= start);
        let Some(entry) = at.checked_sub(1).map(|at| &series[at]) else {
            continue;
        };
        let Ok(end) = series.binary_search_by_key(&exit, |bar| bar.date) else {
            continue;
        };
        if entry.date < start - Duration::days(30) {
            continue;
        }
        let ratio = |date| {
            ratios
                .and_then(|r| r.get(&(ETF_BENCHMARK.to_owned(), date)))
                .copied()
                .unwrap_or(1.0)
        };
        let buy = entry.close * ratio(start);
        let sell = series[end].close * ratio(exit);
        if buy.is_finite() && sell.is_finite() && buy > 0.0 && sell > 0.0 {
            row.etf_return = Some((sell / buy - 1.0) * 100.0);
        }
    }
}

#[cfg(test)]
mod tests;
