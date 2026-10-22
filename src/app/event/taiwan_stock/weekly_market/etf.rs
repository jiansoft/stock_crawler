//! 每週市場摘要的 ETF 一段：全市場淨申購流入、流出最多的 ETF，持有 ETF 的淨申購與折溢價，
//! 以及持有 ETF 這週收到新一期前十大持股時，與前一期相比的新進、剔除與權重變化。
//!
//! 淨申購＝每日受益單位增減 × 當日淨值的合計（[`EtfRepository::weekly_flows`]）；
//! 單位數從 MIS 開始收錄那天才有，歷史回補的日子不計。前十大持股多半每月更新一次，
//! 只有這週才收到新一期的才列（[`EtfRepository::holding_periods`]）。

use std::collections::BTreeSet;

use anyhow::Result;
use chrono::{Days, NaiveDate};

use super::{WEEK_DAYS, hundred_million};
use crate::domain::etf::{
    EtfRepository, EtfWeeklyFlow,
    holding_diff::{HoldingDiff, HoldingWeight, diff_holdings},
};

/// 流入、流出各列幾檔。
const TOP_FLOWS: usize = 5;

/// 權重變化列幾檔。
const TOP_MOVES: usize = 3;

/// 「這週收到」往前看的天數（週六執行，涵蓋週日到週五）。
const RECORDED_DAYS: u64 = 6;

/// 持有 ETF 的前十大持股換期。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct HoldingChange {
    /// ETF 代號。
    pub(super) stock_symbol: String,
    /// 前一期的資料日期。
    pub(super) from: NaiveDate,
    /// 新一期的資料日期。
    pub(super) to: NaiveDate,
    /// 兩期的差異。
    pub(super) diff: HoldingDiff,
}

/// ETF 一段的資料。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct EtfWeekSection {
    /// 期間第一個交易日。
    pub(super) from: NaiveDate,
    /// 期間最後一個交易日。
    pub(super) to: NaiveDate,
    /// 各檔的淨申購與折溢價。
    pub(super) flows: Vec<EtfWeeklyFlow>,
    /// 持有的 ETF 代號（只留這週有資料的）。
    pub(super) held: Vec<String>,
    /// 持有 ETF 這週收到新一期前十大持股的差異（沒有差異的不列）。
    pub(super) changes: Vec<HoldingChange>,
}

/// 讀取最近 [`WEEK_DAYS`] 個交易日的 ETF 資金流向與持有 ETF 的持股換期；沒有任何資金流向時回 `None`。
pub(super) async fn etf_section(
    repo: &impl EtfRepository,
    holdings: &BTreeSet<String>,
    today: NaiveDate,
) -> Result<Option<EtfWeekSection>> {
    let flows = repo.weekly_flows(WEEK_DAYS as i64).await?;
    let (Some(from), Some(to)) = (
        flows.iter().map(|flow| flow.from).min(),
        flows.iter().map(|flow| flow.to).max(),
    ) else {
        return Ok(None);
    };
    let held = flows
        .iter()
        .filter(|flow| holdings.contains(&flow.stock_symbol))
        .map(|flow| flow.stock_symbol.clone())
        .collect();
    let symbols: Vec<String> = holdings.iter().cloned().collect();
    let since = today - Days::new(RECORDED_DAYS);
    let changes = repo
        .holding_periods(&symbols, since)
        .await?
        .into_iter()
        .map(|periods| HoldingChange {
            diff: diff_holdings(&periods.previous.holdings, &periods.current.holdings),
            stock_symbol: periods.stock_symbol,
            from: periods.previous.as_of,
            to: periods.current.as_of,
        })
        .filter(|change| !change.diff.is_empty())
        .collect();
    Ok(Some(EtfWeekSection {
        from,
        to,
        flows,
        held,
        changes,
    }))
}

fn weights(rows: &[HoldingWeight]) -> String {
    rows.iter()
        .map(|row| format!("{} {:.2}%", row.label, row.weight))
        .collect::<Vec<_>>()
        .join("、")
}

/// 一檔持有 ETF 的持股換期：新進、剔除與權重變化最大的幾檔。
fn change_line(change: &HoldingChange, name: &str) -> String {
    let mut parts = Vec::new();
    if !change.diff.added.is_empty() {
        parts.push(format!("新進 {}", weights(&change.diff.added)));
    }
    if !change.diff.removed.is_empty() {
        parts.push(format!("剔除 {}", weights(&change.diff.removed)));
    }
    if !change.diff.moved.is_empty() {
        let moves: Vec<String> = change
            .diff
            .moved
            .iter()
            .take(TOP_MOVES)
            .map(|m| {
                format!(
                    "{} {:.2}% → {:.2}%（{:+.2}）",
                    m.label,
                    m.before,
                    m.after,
                    m.after - m.before
                )
            })
            .collect();
        parts.push(format!("權重變化 {}", moves.join("、")));
    }
    format!(
        "持有 {} {} 前十大持股更新（{} → {}）：{}",
        change.stock_symbol,
        name,
        change.from.format("%m-%d"),
        change.to.format("%m-%d"),
        parts.join("；")
    )
}

/// 「代號 名稱 ±x.x 億」。
fn flow_text(flow: &EtfWeeklyFlow) -> String {
    format!(
        "{} {} {}",
        flow.stock_symbol,
        flow.name,
        hundred_million(flow.net_flow)
    )
}

fn percent(value: Option<f64>) -> String {
    value.map_or_else(|| "無".to_owned(), |v| format!("{v:+.2}%"))
}

/// ETF 一段的各行（開頭為空行）；沒有任何淨申購與持有 ETF 時回空陣列。
pub(super) fn lines(section: &EtfWeekSection) -> Vec<String> {
    let mut inflows: Vec<&EtfWeeklyFlow> =
        section.flows.iter().filter(|f| f.net_flow > 0.0).collect();
    inflows.sort_by(|a, b| b.net_flow.total_cmp(&a.net_flow));
    let mut outflows: Vec<&EtfWeeklyFlow> =
        section.flows.iter().filter(|f| f.net_flow < 0.0).collect();
    outflows.sort_by(|a, b| a.net_flow.total_cmp(&b.net_flow));
    let held: Vec<&EtfWeeklyFlow> = section
        .held
        .iter()
        .filter_map(|symbol| section.flows.iter().find(|f| &f.stock_symbol == symbol))
        .collect();
    if inflows.is_empty() && outflows.is_empty() && held.is_empty() && section.changes.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![
        String::new(),
        format!(
            "ETF 資金流向（{}～{}，受益單位增減 × 淨值）",
            section.from.format("%m-%d"),
            section.to.format("%m-%d")
        ),
    ];
    for (label, rows) in [("淨流入", inflows), ("淨流出", outflows)] {
        if !rows.is_empty() {
            let text: Vec<String> = rows.iter().take(TOP_FLOWS).map(|f| flow_text(f)).collect();
            lines.push(format!("{label}：{}", text.join("、")));
        }
    }
    for flow in held {
        lines.push(format!(
            "持有 {} {}：淨申購 {}，折溢價週平均 {}（最新 {}）",
            flow.stock_symbol,
            flow.name,
            hundred_million(flow.net_flow),
            percent(flow.premium_avg),
            percent(flow.premium_last)
        ));
    }
    for change in &section.changes {
        let name = section
            .flows
            .iter()
            .find(|f| f.stock_symbol == change.stock_symbol)
            .map_or("", |f| f.name.as_str());
        lines.push(change_line(change, name));
    }
    lines
}
