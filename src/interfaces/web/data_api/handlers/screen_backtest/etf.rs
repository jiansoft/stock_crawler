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
mod tests {
    use super::super::{
        portfolio::trading_costs,
        stops::{StopRule, populate_daily_returns},
        summary::summarize,
    };
    use super::*;

    /// 建立固定日期，涵蓋 0050 分割停牌期間。
    fn day(month: u32, day: u32) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2025, month, day).unwrap()
    }

    /// 期末與日頻使用同一組已還原價格，次日開盤仍從市場進場日起算。
    #[test]
    fn suspended_benchmark_matches_daily_endpoint() {
        let series = [(day(6, 10), 25.0), (day(6, 18), 26.0), (day(7, 9), 28.0)]
            .into_iter()
            .map(|(date, close)| PriceBar {
                date,
                close,
                high: close,
                low: close,
            })
            .collect();
        let bars = HashMap::from([(ETF_BENCHMARK.to_owned(), Arc::new(series))]);
        for next_open in [false, true] {
            let mut rows = vec![PeriodRow {
                date: day(6, 11),
                entry_date: Some(day(6, 12)),
                exit_date: Some(day(7, 9)),
                ..Default::default()
            }];
            let ratios =
                next_open.then(|| HashMap::from([((ETF_BENCHMARK.to_owned(), day(7, 9)), 1.1)]));
            fill_missing_returns(&mut rows, &bars, ratios.as_ref());
            let expected = if next_open { 23.2 } else { 12.0 };
            assert!((rows[0].etf_return.unwrap() - expected).abs() < 1e-9);
            let costs = trading_costs(&rows, 1.0);
            populate_daily_returns(
                &mut rows,
                &bars,
                StopRule::default(),
                1.0,
                &costs,
                ratios.as_ref(),
            );
            assert!(rows[0].etf_daily_complete);
            assert!((rows[0].etf_daily_returns.last().unwrap() - expected).abs() < 1e-9);
            assert_eq!(summarize(&rows, &costs, 20).etf_missing_periods, 0);
        }
    }

    /// 超過回看上限、缺出場價及未到期均不得製造有效報酬；既有報酬保留。
    #[test]
    fn missing_or_stale_prices_remain_missing() {
        let bars = HashMap::from([(
            ETF_BENCHMARK.to_owned(),
            Arc::new(vec![
                PriceBar {
                    date: day(6, 10),
                    close: 25.0,
                    high: 25.0,
                    low: 25.0,
                },
                PriceBar {
                    date: day(7, 15),
                    close: 28.0,
                    high: 28.0,
                    low: 28.0,
                },
            ]),
        )]);
        let mut rows = vec![
            PeriodRow {
                date: day(7, 11),
                exit_date: Some(day(7, 15)),
                ..Default::default()
            },
            PeriodRow {
                date: day(6, 11),
                exit_date: Some(day(7, 9)),
                ..Default::default()
            },
            PeriodRow {
                date: day(6, 11),
                exit_date: Some(day(7, 15)),
                pending: true,
                ..Default::default()
            },
            PeriodRow {
                date: day(6, 11),
                exit_date: Some(day(7, 15)),
                etf_return: Some(5.0),
                ..Default::default()
            },
        ];
        fill_missing_returns(&mut rows, &bars, None);
        assert_eq!(
            rows.iter().map(|r| r.etf_return).collect::<Vec<_>>(),
            [None, None, None, Some(5.0)]
        );
    }
}
