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

use anyhow::{Result, bail};
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

/// 每年度 F 分數歷史從這一年起算（前一年度只當比較基準）；Yahoo 年報大約從這時開始齊全。
const HISTORY_SINCE_YEAR: i32 = 2012;

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
#[derive(Debug)]
pub(crate) struct Rebuilt {
    pub(crate) summary: RebuildSummary,
    pub(crate) snapshots: Vec<FinancialHealthSnapshot>,
}

/// 排程入口：以今天的年度重算並寫入，再通知持股的分數變化。
///
/// 讀不到上一次結果時照常重算、只是不通知；通知失敗不影響重算結果。
pub async fn execute() -> Result<()> {
    run(Local::now().year()).await
}

/// [`execute`] 的本體：以 `current_year` 為評分基準年度重算並通知。
async fn run(current_year: i32) -> Result<()> {
    let repo = PgFinancialHealthRepository::new();
    let previous = repo.fetch_financial_health().await.unwrap_or_else(|why| {
        tracing::warn!("讀取上一次財務體質失敗，本次不發持股通知: {why:#}");
        Vec::new()
    });
    let rebuilt = rebuild(&repo, current_year).await?;
    tracing::info!("財務體質重算結束: {}", rebuilt.summary);
    // 歷史分數只給回測用，失敗不影響今天的財務體質與通知。
    match rebuild_f_score_history(&repo, HISTORY_SINCE_YEAR).await {
        Ok(written) => tracing::info!("F 分數歷史重算結束: rows={written}"),
        Err(why) => tracing::warn!("F 分數歷史重算失敗: {why:#}"),
    }

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
///
/// # Errors
///
/// 讀取或寫入失敗，或一檔都算不出來（此時不清空舊結果）時回傳錯誤。
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
    // 整批取代會先清空整張表；讀到 0 檔多半是上游財報表異常，保留上一次結果。
    if snapshots.is_empty() {
        bail!("財務體質重算 0 檔（年報、季報都讀不到），不清空 financial_health");
    }
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

/// 讀取 `since_year` 前一年起的年報，逐檔逐年算出 F 分數，整批取代 `financial_f_score_history`；
/// 回傳寫入列數。
///
/// # Errors
///
/// 讀取或寫入失敗，或一年都算不出來（此時不清空舊結果）時回傳錯誤。
pub(crate) async fn rebuild_f_score_history(
    repo: &impl FinancialHealthRepository,
    since_year: i32,
) -> Result<u64> {
    let mut by_stock: BTreeMap<String, BTreeMap<i32, AnnualFigures>> = BTreeMap::new();
    for (symbol, year, figures) in repo.fetch_annual_figures(since_year - 1).await? {
        by_stock.entry(symbol).or_default().insert(year, figures);
    }
    let scores: Vec<(String, health::YearlyFScore)> = by_stock
        .into_iter()
        .flat_map(|(symbol, annual)| {
            health::yearly_f_scores(&annual)
                .into_iter()
                .filter(|score| score.fiscal_year >= since_year)
                .map(move |score| (symbol.clone(), score))
        })
        .collect();
    if scores.is_empty() {
        bail!("F 分數歷史重算 0 列（讀不到年報），不清空 financial_f_score_history");
    }
    repo.replace_f_score_history(&scores).await
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
