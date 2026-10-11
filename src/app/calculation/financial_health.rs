//! 每日重算全市場財務體質（年度 F 分數與選股用的最新一季指標），整批寫入 `financial_health`。
//!
//! 評分規則在 [`crate::domain::financial::health`]，與 Data API 的單檔查詢共用。
//! 排程在 06:00 Yahoo 三大財務報表採集之後執行；`/stocks/screen` 以這張表篩選與排序。
//! 重算前先讀出上一次的結果，重算後比對持股、發送 F 分數變化通知
//! （[`crate::app::event::taiwan_stock::health_alert`]）。

use std::{
    collections::{BTreeMap, HashMap},
    fmt,
};

use anyhow::Result;
use chrono::{Datelike, Local};

use crate::{
    app::event::taiwan_stock::{health_alert, holdings},
    core::alert,
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

/// 一輪重算的統計與寫入的快照。
pub(crate) struct Rebuilt {
    pub(crate) summary: RebuildSummary,
    pub(crate) snapshots: Vec<FinancialHealthSnapshot>,
}

/// 排程入口：以今天的年度重算並寫入，再通知持股的分數變化。
///
/// 讀不到上一次結果時照常重算、只是不通知；通知失敗不影響重算結果。
pub async fn execute() -> Result<()> {
    let repo = PgFinancialHealthRepository::new();
    let previous = repo.fetch_financial_health().await.unwrap_or_else(|why| {
        tracing::warn!("讀取上一次財務體質失敗，本次不發持股通知: {why:#}");
        Vec::new()
    });
    let rebuilt = rebuild(&repo, Local::now().year()).await?;
    tracing::info!("財務體質重算結束: {}", rebuilt.summary);

    match holdings::common_stock_holdings().await {
        Ok(held) => {
            if let Some(message) = health_alert::build_notification(
                &previous,
                &rebuilt.snapshots,
                &held,
                holdings::stock_name,
            ) {
                alert::send_message(&message).await;
            }
        }
        Err(why) => tracing::warn!("取得持股失敗，略過財務體質變化通知: {why:#}"),
    }
    Ok(())
}

/// 讀取年報與季報數值、逐檔算出財務體質，整批取代 `financial_health`。
pub(crate) async fn rebuild(
    repo: &impl FinancialHealthRepository,
    current_year: i32,
) -> Result<Rebuilt> {
    let annual = repo
        .fetch_annual_figures(current_year - ANNUAL_LOOKBACK_YEARS)
        .await?;
    let quarters = repo
        .fetch_quarter_figures(current_year - QUARTER_LOOKBACK_YEARS)
        .await?;
    let snapshots = build_snapshots(annual, quarters);
    let written = repo.replace_financial_health(&snapshots).await?;

    Ok(Rebuilt {
        summary: RebuildSummary {
            stocks: snapshots.len(),
            scored: snapshots.iter().filter(|s| s.f_score.is_some()).count(),
            with_quarter: snapshots.iter().filter(|s| s.quarter.is_some()).count(),
            written,
        },
        snapshots,
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
