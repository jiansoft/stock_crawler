//! 每週市場摘要的 ETF 一段：全市場淨申購流入、流出最多的 ETF，與持有 ETF 的淨申購與折溢價。
//!
//! 淨申購＝每日受益單位增減 × 當日淨值的合計（[`EtfRepository::weekly_flows`]）；
//! 單位數從 MIS 開始收錄那天才有，歷史回補的日子不計。

use std::collections::BTreeSet;

use anyhow::Result;
use chrono::NaiveDate;

use super::{WEEK_DAYS, hundred_million};
use crate::domain::etf::{EtfRepository, EtfWeeklyFlow};

/// 流入、流出各列幾檔。
const TOP_FLOWS: usize = 5;

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
}

/// 讀取最近 [`WEEK_DAYS`] 個交易日的 ETF 資金流向；沒有任何資料時回 `None`。
pub(super) async fn etf_section(
    repo: &impl EtfRepository,
    holdings: &BTreeSet<String>,
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
    Ok(Some(EtfWeekSection {
        from,
        to,
        flows,
        held,
    }))
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
    if inflows.is_empty() && outflows.is_empty() && held.is_empty() {
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
    lines
}
