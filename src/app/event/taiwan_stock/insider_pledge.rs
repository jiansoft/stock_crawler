//! # 持股董監質押變動通知
//!
//! 每月公開資訊觀測站彙整內部人（董事、監察人、經理人、大股東）的持股與設質後，
//! 比對持股普通股的前後兩期，把設質增減與持股大幅增減彙整成一則 Telegram。
//!
//! ## 流程（每月 10–28 日 20:45 排程）
//!
//! 1. 取目前持股的普通股代號（[`holdings::common_stock_holdings`]）。
//! 2. 抓上市、上櫃最新月份的董監事持股明細；一個市場失敗不影響另一個市場。
//! 3. 同一人多個職稱合併成一筆（職稱以「、」串接，股數取最大）。
//! 4. 與 Redis 保存的前一期（`insider_pledge:snapshot:{代號}`）比較；月份相同代表已處理過，略過。
//!    - 有前一期：列出設質增減、關係人設質增減，以及持股增減達
//!      [`HOLDING_CHANGE_MIN_SHARES`] 股且達前一期 [`HOLDING_CHANGE_MIN_PERCENT`]% 的人。
//!      新上任者只在有設質時列出；卸任者不列。
//!    - 沒有前一期（第一次執行或 Redis 資料遺失）：列出目前有設質的人，作為追蹤基準。
//! 5. 有內容才發訊息；送出後寫回這一期。
//!
//! 開放資料只有最新一個月份、約每月 18 日出表，因此排程只在 10–28 日跑，
//! 出表後第一次執行就會通知，之後同月份都略過。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::{add_thousand_separators, format_decimal_with_commas, holdings};
use crate::{
    core::{alert, util::text},
    infra::{
        crawler::mops::insider_holding::{self, InsiderHolding},
        nosql::redis::{CLIENT, RedisError},
    },
};

/// 前一期資料的保存時間（400 天）；即使某個月份開放資料延遲，也比得到再前一期。
const SNAPSHOT_TTL_SECONDS: usize = 60 * 60 * 24 * 400;

/// 持股增減通知門檻：至少 100 張。
pub const HOLDING_CHANGE_MIN_SHARES: i64 = 100_000;

/// 持股增減通知門檻：至少為前一期持股的 5%。
pub const HOLDING_CHANGE_MIN_PERCENT: i64 = 5;

/// 單一內部人（同名多職稱已合併）的持股與設質。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Insider {
    /// 姓名（法人董事為公司名稱）。
    name: String,
    /// 職稱，依資料出現順序。
    titles: Vec<String>,
    /// 目前持股（股）。
    shares: i64,
    /// 設質股數（股）。
    pledged: i64,
    /// 內部人關係人設質股數（股）。
    related_pledged: i64,
}

/// 單一股票單一月份的內部人持股，也是存進 Redis 的前一期資料。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Snapshot {
    /// 資料月份（該月 1 日）。
    month: NaiveDate,
    /// 內部人，依資料出現順序。
    insiders: Vec<Insider>,
}

/// 要通知的一項變動。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Change {
    /// 本人設質股數變動。
    Pledged { insider: Insider, before: i64 },
    /// 關係人設質股數變動。
    RelatedPledged { insider: Insider, before: i64 },
    /// 持股大幅增減。
    Shares { insider: Insider, before: i64 },
    /// 沒有前一期時，列出目前有設質的人作為基準。
    Current(Insider),
}

/// 單一股票的通知內容。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Report {
    stock_symbol: String,
    month: NaiveDate,
    /// 沒有前一期、這次只建立基準。
    baseline: bool,
    changes: Vec<Change>,
}

/// 一輪通知的結果。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RunSummary {
    /// 檢查的持股檔數。
    pub holdings: usize,
    /// 開放資料中有內部人資料的持股檔數。
    pub covered: usize,
    /// 月份與前一期相同、這次略過的檔數。
    pub unchanged: usize,
    /// 這次建立基準（沒有前一期）的檔數。
    pub baseline: usize,
    /// 有變動、放進通知的檔數（含基準中有設質者）。
    pub reported: usize,
    /// 讀取前一期失敗而略過的檔數。
    pub failed: usize,
}

/// 排程入口：比對持股的董監持股與設質，有變動就通知。
///
/// 開放資料或單檔處理失敗只記錄、不中斷；只有查不到持股時回傳錯誤。
pub async fn execute() -> Result<()> {
    let symbols = holdings::common_stock_holdings()
        .await
        .context("fetch active holdings for insider pledge notification failed")?;

    let mut rows = Vec::new();
    for (market, result) in [
        ("上市", insider_holding::visit_listed().await),
        ("上櫃", insider_holding::visit_otc().await),
    ] {
        match result {
            Ok(mut market_rows) => rows.append(&mut market_rows),
            Err(why) => tracing::warn!("{market}董監持股明細抓取失敗: {why:#}"),
        }
    }

    let snapshots = group_snapshots(rows, &symbols);
    let mut summary = RunSummary {
        holdings: symbols.len(),
        covered: snapshots.len(),
        ..Default::default()
    };
    let mut reports = Vec::new();
    let mut to_save = Vec::new();
    for (symbol, current) in snapshots {
        let previous = match load_snapshot(&symbol).await {
            Ok(previous) => previous,
            Err(why) => {
                // 讀不到前一期就無法判斷變動；當成基準會在 Redis 恢復後重複通知，因此整檔略過。
                summary.failed += 1;
                tracing::warn!("讀取董監持股前一期失敗，略過: stock_symbol={symbol}, error={why}");
                continue;
            }
        };
        if previous
            .as_ref()
            .is_some_and(|previous| previous.month == current.month)
        {
            summary.unchanged += 1;
            continue;
        }

        let report = compare(&symbol, previous.as_ref(), &current);
        if report.baseline {
            summary.baseline += 1;
        }
        if !report.changes.is_empty() {
            summary.reported += 1;
            reports.push(report);
        }
        to_save.push((symbol, current));
    }

    if !reports.is_empty() {
        alert::send_message(&build_message(&reports, holdings::stock_name)).await;
    }
    for (symbol, snapshot) in &to_save {
        save_snapshot(symbol, snapshot).await;
    }

    tracing::info!(
        "持股董監質押通知結束: holdings={}, covered={}, unchanged={}, baseline={}, reported={}, failed={}",
        summary.holdings,
        summary.covered,
        summary.unchanged,
        summary.baseline,
        summary.reported,
        summary.failed
    );
    Ok(())
}

/// 依股票分組，只留持股；同一人多個職稱合併成一筆。
fn group_snapshots(
    rows: Vec<InsiderHolding>,
    symbols: &BTreeSet<String>,
) -> BTreeMap<String, Snapshot> {
    let mut snapshots: BTreeMap<String, Snapshot> = BTreeMap::new();
    for row in rows {
        if !symbols.contains(&row.stock_symbol) {
            continue;
        }
        let snapshot = snapshots
            .entry(row.stock_symbol.clone())
            .or_insert_with(|| Snapshot {
                month: row.month,
                insiders: Vec::new(),
            });
        snapshot.month = snapshot.month.max(row.month);

        match snapshot
            .insiders
            .iter_mut()
            .find(|insider| insider.name == row.name)
        {
            // 同名多列通常是同一人兼任多職、數字相同；極少數是同名不同人，取最大值為近似。
            Some(insider) => {
                if !insider.titles.contains(&row.title) {
                    insider.titles.push(row.title);
                }
                insider.shares = insider.shares.max(row.shares);
                insider.pledged = insider.pledged.max(row.pledged);
                insider.related_pledged = insider.related_pledged.max(row.related_pledged);
            }
            None => snapshot.insiders.push(Insider {
                name: row.name,
                titles: vec![row.title],
                shares: row.shares,
                pledged: row.pledged,
                related_pledged: row.related_pledged,
            }),
        }
    }
    snapshots
}

/// 比較前後兩期；沒有前一期時列出目前有設質的人作為基準。
fn compare(stock_symbol: &str, previous: Option<&Snapshot>, current: &Snapshot) -> Report {
    let changes = match previous {
        Some(previous) => diff(&previous.insiders, &current.insiders),
        None => current
            .insiders
            .iter()
            .filter(|insider| insider.pledged > 0 || insider.related_pledged > 0)
            .cloned()
            .map(Change::Current)
            .collect(),
    };
    Report {
        stock_symbol: stock_symbol.to_string(),
        month: current.month,
        baseline: previous.is_none(),
        changes,
    }
}

/// 逐人比較設質、關係人設質與持股。
fn diff(previous: &[Insider], current: &[Insider]) -> Vec<Change> {
    let mut changes = Vec::new();
    for insider in current {
        let before = previous.iter().find(|before| before.name == insider.name);
        let (pledged, related_pledged) =
            before.map_or((0, 0), |before| (before.pledged, before.related_pledged));
        if insider.pledged != pledged {
            changes.push(Change::Pledged {
                insider: insider.clone(),
                before: pledged,
            });
        }
        if insider.related_pledged != related_pledged {
            changes.push(Change::RelatedPledged {
                insider: insider.clone(),
                before: related_pledged,
            });
        }
        // 新上任者的持股不算「增減」。
        if let Some(before) = before
            && is_significant_holding_change(before.shares, insider.shares)
        {
            changes.push(Change::Shares {
                insider: insider.clone(),
                before: before.shares,
            });
        }
    }
    changes
}

/// 持股增減是否達通知門檻：張數與比例都要達到。
fn is_significant_holding_change(before: i64, after: i64) -> bool {
    let delta = (after - before).abs();
    delta >= HOLDING_CHANGE_MIN_SHARES
        && (before == 0 || delta * 100 >= before * HOLDING_CHANGE_MIN_PERCENT)
}

/// 組出通知訊息（Telegram MarkdownV2）。
///
/// `stock_name` 由呼叫端提供（正式流程讀股票主檔快取），測試時可換成固定對照。
fn build_message(reports: &[Report], stock_name: impl Fn(&str) -> String) -> String {
    let month = reports
        .iter()
        .map(|report| report.month)
        .max()
        .unwrap_or_default();
    let mut message = String::with_capacity(256 + reports.len() * 256);
    let _ = writeln!(
        &mut message,
        "{}",
        text::escape_markdown_v2(format!(
            "持股董監質押變動︰{} 年 {} 月",
            month.year(),
            month.month()
        ))
    );
    for report in reports {
        let mut heading = format!(
            "【{} {}】",
            report.stock_symbol,
            stock_name(&report.stock_symbol)
        );
        if report.baseline {
            heading.push_str("首次追蹤，目前設質");
        }
        let _ = write!(&mut message, "\n{}\n", text::escape_markdown_v2(heading));
        for change in &report.changes {
            let _ = writeln!(
                &mut message,
                "{}",
                text::escape_markdown_v2(describe(change))
            );
        }
    }
    message
}

/// 一項變動的文字。
fn describe(change: &Change) -> String {
    match change {
        Change::Pledged { insider, before } => format!(
            "{} 設質 {} → {}{}",
            who(insider),
            lots(*before),
            lots(insider.pledged),
            pledge_ratio(insider)
        ),
        Change::RelatedPledged { insider, before } => format!(
            "{} 關係人設質 {} → {}",
            who(insider),
            lots(*before),
            lots(insider.related_pledged)
        ),
        Change::Shares { insider, before } => format!(
            "{} 持股 {} → {}",
            who(insider),
            lots(*before),
            lots(insider.shares)
        ),
        Change::Current(insider) => {
            let mut parts = Vec::new();
            if insider.pledged > 0 {
                parts.push(format!(
                    "設質 {}{}",
                    lots(insider.pledged),
                    pledge_ratio(insider)
                ));
            }
            if insider.related_pledged > 0 {
                parts.push(format!("關係人設質 {}", lots(insider.related_pledged)));
            }
            format!("{} {}", who(insider), parts.join("、"))
        }
    }
}

/// 「姓名（職稱、職稱）」。
fn who(insider: &Insider) -> String {
    format!("{}（{}）", insider.name, insider.titles.join("、"))
}

/// 設質佔持股比例，例如「（佔持股 77.4%）」；沒有持股時不顯示。
fn pledge_ratio(insider: &Insider) -> String {
    if insider.shares <= 0 || insider.pledged <= 0 {
        return String::new();
    }
    let ratio =
        Decimal::from(insider.pledged) * Decimal::ONE_HUNDRED / Decimal::from(insider.shares);
    format!("（佔持股 {}%）", format_decimal_with_commas(ratio))
}

/// 股數轉成張（四捨五入）；不足一張時顯示股數。
fn lots(shares: i64) -> String {
    if shares != 0 && shares.abs() < 1_000 {
        return format!("{shares} 股");
    }
    let lots = (shares + shares.signum() * 500) / 1_000;
    format!("{} 張", add_thousand_separators(&lots.to_string()))
}

/// 前一期資料的 Redis key。
fn snapshot_key(stock_symbol: &str) -> String {
    format!("insider_pledge:snapshot:{stock_symbol}")
}

/// 讀取前一期；不存在時回傳 `None`。
async fn load_snapshot(stock_symbol: &str) -> Result<Option<Snapshot>, RedisError> {
    match CLIENT.get_bytes(&snapshot_key(stock_symbol)).await {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|why| RedisError::Parse(why.to_string())),
        Err(RedisError::NotFound) => Ok(None),
        Err(why) => Err(why),
    }
}

/// 寫回這一期；失敗只記錄，下次排程會再當成新月份處理而重複通知。
async fn save_snapshot(stock_symbol: &str, snapshot: &Snapshot) {
    let key = snapshot_key(stock_symbol);
    let json = match serde_json::to_string(snapshot) {
        Ok(json) => json,
        Err(why) => {
            tracing::warn!("序列化董監持股 {key} 失敗: {why}");
            return;
        }
    };
    if let Err(why) = CLIENT.set(&key, json, SNAPSHOT_TTL_SECONDS).await {
        tracing::warn!("寫入董監持股 {key} 失敗，下次排程可能重複通知: {why:?}");
    }
}

#[cfg(test)]
mod tests;
