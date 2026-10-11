//! 每日重算全市場財務體質（年度 F 分數與選股用的最新一季指標），整批寫入 `financial_health`。
//!
//! 評分規則在 [`crate::domain::financial::health`]，與 Data API 的單檔查詢共用。
//! 排程在 06:00 Yahoo 三大財務報表採集之後執行；`/stocks/screen` 以這張表篩選與排序。

use std::{
    collections::{BTreeMap, HashMap},
    fmt,
};

use anyhow::Result;
use chrono::{Datelike, Local};

use crate::{
    domain::financial::{
        health::{self, AnnualFigures, FinancialHealthSnapshot, QuarterFigures},
        repository::FinancialHealthRepository,
    },
    infra::database::repository::financial_health::PgFinancialHealthRepository,
};

/// 年報往回讀幾年：評分年度與前一年度，加上一年緩衝（年報公布前的 1–3 月）。
const ANNUAL_LOOKBACK_YEARS: i32 = 3;
/// 季報往回讀幾年：最新一季往前推四季可能跨年，再加一年緩衝。
const QUARTER_LOOKBACK_YEARS: i32 = 2;

/// 一輪重算的統計。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RebuildSummary {
    /// 有任何財報數值的股票檔數。
    pub stocks: usize,
    /// 算得出 F 分數（有評分年度）的檔數。
    pub scored: usize,
    /// 有最新一季指標的檔數。
    pub with_quarter: usize,
    /// 寫入列數。
    pub written: u64,
}

impl fmt::Display for RebuildSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "stocks={} scored={} with_quarter={} written={}",
            self.stocks, self.scored, self.with_quarter, self.written
        )
    }
}

/// 排程入口：以今天的年度重算並寫入。
pub async fn execute() -> Result<()> {
    let summary = rebuild(&PgFinancialHealthRepository::new(), Local::now().year()).await?;
    tracing::info!("財務體質重算結束: {summary}");
    Ok(())
}

/// 讀取年報與季報數值、逐檔算出財務體質，整批取代 `financial_health`。
pub(crate) async fn rebuild(
    repo: &impl FinancialHealthRepository,
    current_year: i32,
) -> Result<RebuildSummary> {
    let annual = repo
        .fetch_annual_figures(current_year - ANNUAL_LOOKBACK_YEARS)
        .await?;
    let quarters = repo
        .fetch_quarter_figures(current_year - QUARTER_LOOKBACK_YEARS)
        .await?;
    let snapshots = build_snapshots(annual, quarters);
    let written = repo.replace_financial_health(&snapshots).await?;

    Ok(RebuildSummary {
        stocks: snapshots.len(),
        scored: snapshots.iter().filter(|s| s.f_score.is_some()).count(),
        with_quarter: snapshots.iter().filter(|s| s.quarter.is_some()).count(),
        written,
    })
}

/// 依股票分組後逐檔算出財務體質，依股票代號排序。
fn build_snapshots(
    annual: Vec<(String, i32, AnnualFigures)>,
    quarters: Vec<(String, QuarterFigures)>,
) -> Vec<FinancialHealthSnapshot> {
    let mut by_stock: BTreeMap<String, (BTreeMap<i32, AnnualFigures>, Vec<QuarterFigures>)> =
        BTreeMap::new();
    for (symbol, year, figures) in annual {
        by_stock.entry(symbol).or_default().0.insert(year, figures);
    }
    let mut quarters_by_stock: HashMap<String, Vec<QuarterFigures>> = HashMap::new();
    for (symbol, figures) in quarters {
        quarters_by_stock.entry(symbol).or_default().push(figures);
    }
    for (symbol, figures) in quarters_by_stock {
        by_stock.entry(symbol).or_default().1 = figures;
    }

    by_stock
        .into_iter()
        .map(|(symbol, (annual, quarters))| health::snapshot(symbol, &annual, &quarters))
        .collect()
}

#[cfg(test)]
mod tests;
