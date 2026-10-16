//! 每週市場摘要的產業相對強度一段：成分股 RS 中位數最強、最弱，與一週前相比轉強、轉弱最多的產業。

use std::collections::BTreeMap;

use anyhow::Result;
use chrono::NaiveDate;

use super::{TOP, WEEK_DAYS, signed};
use crate::domain::quote::{
    forward_return::ForwardReturnRepository,
    relative_strength::{ScoredUniverseQuery, group_by_industry, median_score},
};

/// 產業至少要有幾檔計分股票才列入（檔數太少的中位數不穩定）。
pub(super) const MIN_INDUSTRY_STOCKS: usize = 5;

/// 產業 RS 中位數與一週變化。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct IndustryScore {
    pub(super) industry: String,
    pub(super) median: f64,
    /// 與一週前中位數的差；一週前沒有該產業時為 `None`。
    pub(super) change: Option<f64>,
}

/// 產業 RS 一段：資料日與各產業，由強到弱。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct IndustrySection {
    pub(super) data_date: NaiveDate,
    pub(super) industries: Vec<IndustryScore>,
}

/// 最新資料日與 [`WEEK_DAYS`] 個交易日前的產業 RS 中位數；沒有分數回 `None`。
pub(super) async fn industry_section(
    quote: &(impl ScoredUniverseQuery + ForwardReturnRepository),
    date: NaiveDate,
) -> Result<Option<IndustrySection>> {
    let Some((data_date, current)) = quote.fetch_scored_universe(date).await? else {
        return Ok(None);
    };
    let dates = quote.fetch_trading_dates(data_date, WEEK_DAYS + 1).await?;
    let prior = match dates.get(WEEK_DAYS) {
        Some(&prior_until) => quote.fetch_scored_universe(prior_until).await?,
        None => None,
    };
    let prior_medians: BTreeMap<String, f64> = prior
        .map(|(_, scored)| {
            group_by_industry(scored, 0)
                .into_iter()
                .filter_map(|(industry, stocks)| Some((industry, median_score(&stocks)?)))
                .collect()
        })
        .unwrap_or_default();
    let mut industries: Vec<IndustryScore> = group_by_industry(current, 0)
        .into_iter()
        .filter(|(_, stocks)| stocks.len() >= MIN_INDUSTRY_STOCKS)
        .filter_map(|(industry, stocks)| {
            let median = median_score(&stocks)?;
            Some(IndustryScore {
                change: prior_medians.get(&industry).map(|prior| median - prior),
                industry,
                median,
            })
        })
        .collect();
    industries.sort_by(|a, b| {
        b.median
            .total_cmp(&a.median)
            .then_with(|| a.industry.cmp(&b.industry))
    });
    Ok(Some(IndustrySection {
        data_date,
        industries,
    }))
}

/// 產業相對強度一段的各行（開頭為空行）；呼叫端確保至少有一個產業。
pub(super) fn lines(section: &IndustrySection) -> Vec<String> {
    let list = |items: &mut dyn Iterator<Item = &IndustryScore>, with_change: bool| {
        items
            .take(TOP)
            .map(|item| match (with_change, item.change) {
                (true, Some(change)) => {
                    format!("{} {}（{}）", item.industry, item.median, signed(change))
                }
                _ => format!("{} {}", item.industry, item.median),
            })
            .collect::<Vec<_>>()
            .join("、")
    };
    let mut lines = vec![String::new()];
    lines.push(format!(
        "產業相對強度（成分股 RS 中位數，括號為與 {WEEK_DAYS} 個交易日前相比）"
    ));
    lines.push(format!(
        "• 最強：{}",
        list(&mut section.industries.iter(), true)
    ));
    lines.push(format!(
        "• 最弱：{}",
        list(&mut section.industries.iter().rev(), true)
    ));
    let mut changed: Vec<(&str, f64)> = section
        .industries
        .iter()
        .filter_map(|item| Some((item.industry.as_str(), item.change?)))
        .collect();
    changed.sort_by(|(a_name, a), (b_name, b)| b.total_cmp(a).then_with(|| a_name.cmp(b_name)));
    let movers = |items: &mut dyn Iterator<Item = &(&str, f64)>| {
        items
            .take(TOP)
            .map(|(industry, change)| format!("{industry} {}", signed(*change)))
            .collect::<Vec<_>>()
            .join("、")
    };
    let up = movers(&mut changed.iter().filter(|(_, change)| *change > 0.0));
    if !up.is_empty() {
        lines.push(format!("• 轉強：{up}"));
    }
    let down = movers(&mut changed.iter().rev().filter(|(_, change)| *change < 0.0));
    if !down.is_empty() {
        lines.push(format!("• 轉弱：{down}"));
    }
    lines
}
