//! 每週市場摘要的法人產業資金一段：外資、投信近一週在各產業的估計淨買賣，淨流入與淨流出最多的產業。

use std::collections::BTreeMap;

use super::{TOP, hundred_million};
use crate::domain::chip::flow::{FlowPeriod, StockFlow};

/// 單一法人的產業淨買賣一段：期間與各產業淨額（元），由多到少。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct FlowSection {
    pub(super) label: &'static str,
    pub(super) period: FlowPeriod,
    pub(super) industries: Vec<(String, f64)>,
}

/// 依產業加總估計淨買賣金額（未分類略過），由多到少。
pub(super) fn flow_section(
    label: &'static str,
    period: FlowPeriod,
    rows: Vec<StockFlow>,
) -> FlowSection {
    let mut totals: BTreeMap<String, f64> = BTreeMap::new();
    for row in rows {
        if !row.industry.is_empty() {
            *totals.entry(row.industry).or_default() += row.net_amount;
        }
    }
    let mut industries: Vec<(String, f64)> = totals.into_iter().collect();
    industries.sort_by(|(a_name, a), (b_name, b)| b.total_cmp(a).then_with(|| a_name.cmp(b_name)));
    FlowSection {
        label,
        period,
        industries,
    }
}

/// 單一法人產業淨買賣一段的各行（開頭為空行）；呼叫端確保至少有一個產業。
pub(super) fn lines(section: &FlowSection) -> Vec<String> {
    let list = |items: &mut dyn Iterator<Item = &(String, f64)>| {
        items
            .take(TOP)
            .map(|(industry, amount)| format!("{industry} {}", hundred_million(*amount)))
            .collect::<Vec<_>>()
            .join("、")
    };
    let mut lines = vec![String::new()];
    lines.push(format!(
        "{}近 {} 日產業淨買賣（{}～{}）",
        section.label, section.period.days, section.period.from, section.period.data_date
    ));
    let inflow = list(
        &mut section
            .industries
            .iter()
            .filter(|(_, amount)| *amount > 0.0),
    );
    lines.push(format!(
        "• 流入：{}",
        if inflow.is_empty() { "無" } else { &inflow }
    ));
    let outflow = list(
        &mut section
            .industries
            .iter()
            .rev()
            .filter(|(_, amount)| *amount < 0.0),
    );
    lines.push(format!(
        "• 流出：{}",
        if outflow.is_empty() { "無" } else { &outflow }
    ));
    lines
}
