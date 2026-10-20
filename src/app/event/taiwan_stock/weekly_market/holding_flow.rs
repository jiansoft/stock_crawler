//! 每週市場摘要的持股法人與主力一段：外資、投信、主力近一週在持股（普通股）的累計買賣超，買超、
//! 賣超最多的幾檔。
//!
//! 法人直接沿用產業資金一段已載入的全體個股淨買賣（[`StockFlow`]），只篩出持股；主力另外查
//! `broker_flow` 同一期間的合計（[`BrokerPeriodNet`]），佔量與名稱借用法人資料的同期成交量。

use std::collections::BTreeSet;

use super::TOP;
use crate::domain::{
    chip::flow::{FlowPeriod, StockFlow},
    portfolio::chip::BrokerPeriodNet,
};

/// 單一法人在一檔持股的期間累計買賣超。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct HoldingFlow {
    pub(super) stock_symbol: String,
    pub(super) name: String,
    /// 期間累計淨張數（四捨五入，正為買超）。
    pub(super) net_lots: i64,
    /// 估計淨買賣金額（元），排序用，與產業資金一段一致。
    pub(super) net_amount: f64,
    /// 淨股數佔期間成交量（%）；沒有成交量為 `None`。
    pub(super) share: Option<f64>,
    /// 截至資料日的連續天數：正數為連續買超、負數為連續賣超。
    pub(super) streak: i32,
    /// 期間內買超、賣超的天數（主力用；法人以連續天數表示，為 `None`）。
    pub(super) buy_sell_days: Option<(i64, i64)>,
}

/// 單一法人的持股買賣超一段。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct HoldingFlowSection {
    pub(super) label: &'static str,
    pub(super) period: FlowPeriod,
    /// 淨張數不為 0 的持股，依估計淨買賣金額由多到少。
    pub(super) flows: Vec<HoldingFlow>,
}

/// 從全體個股淨買賣篩出持股；四捨五入後不到 1 張的略過。
pub(super) fn holding_flow_section(
    label: &'static str,
    period: FlowPeriod,
    rows: &[StockFlow],
    symbols: &BTreeSet<String>,
) -> HoldingFlowSection {
    let mut flows: Vec<HoldingFlow> = rows
        .iter()
        .filter(|row| symbols.contains(&row.stock_symbol))
        .filter_map(|row| {
            let net_lots = (row.net_shares + row.net_shares.signum() * 500) / 1000;
            (net_lots != 0).then(|| HoldingFlow {
                stock_symbol: row.stock_symbol.clone(),
                name: row.name.clone(),
                net_lots,
                net_amount: row.net_amount,
                share: row
                    .volume
                    .filter(|volume| *volume > 0.0)
                    .map(|volume| row.net_shares as f64 / volume * 100.0),
                streak: row.streak,
                buy_sell_days: None,
            })
        })
        .collect();
    sort_by_amount(&mut flows);
    HoldingFlowSection {
        label,
        period,
        flows,
    }
}

/// 主力一段：期間主力合計（張）；名稱、期間成交量與收盤價取自同期法人資料 `rows`，找不到時以代號
/// 為名、不算佔量，排序金額以 0 計。
pub(super) fn broker_flow_section(
    period: FlowPeriod,
    nets: &[BrokerPeriodNet],
    rows: &[StockFlow],
) -> HoldingFlowSection {
    let mut flows: Vec<HoldingFlow> = nets
        .iter()
        .filter(|net| net.main_net != 0)
        .map(|net| {
            let row = rows.iter().find(|row| row.stock_symbol == net.stock_symbol);
            let shares = net.main_net as f64 * 1000.0;
            HoldingFlow {
                stock_symbol: net.stock_symbol.clone(),
                name: row.map_or_else(|| net.stock_symbol.clone(), |row| row.name.clone()),
                net_lots: net.main_net,
                net_amount: shares * row.and_then(|row| row.closing_price).unwrap_or_default(),
                share: row
                    .and_then(|row| row.volume)
                    .filter(|volume| *volume > 0.0)
                    .map(|volume| shares / volume * 100.0),
                streak: 0,
                buy_sell_days: Some((net.buy_days, net.sell_days)),
            }
        })
        .collect();
    sort_by_amount(&mut flows);
    HoldingFlowSection {
        label: "主力",
        period,
        flows,
    }
}

/// 依估計淨買賣金額由多到少，同額依代號。
fn sort_by_amount(flows: &mut [HoldingFlow]) {
    flows.sort_by(|a, b| {
        b.net_amount
            .total_cmp(&a.net_amount)
            .then_with(|| a.stock_symbol.cmp(&b.stock_symbol))
    });
}

/// 一檔持股的文字：代號、名稱、張數，括號內為佔量與連續天數。
fn flow_text(flow: &HoldingFlow) -> String {
    let mut details = Vec::new();
    if let Some(share) = flow.share {
        details.push(format!("佔量 {share:+.1}%"));
    }
    match flow.streak {
        days if days > 0 => details.push(format!("連買 {days} 日")),
        days if days < 0 => details.push(format!("連賣 {} 日", -days)),
        _ => {}
    }
    if let Some((buy, sell)) = flow.buy_sell_days {
        details.push(format!("買超 {buy} 日、賣超 {sell} 日"));
    }
    let mut text = format!("{} {} {:+} 張", flow.stock_symbol, flow.name, flow.net_lots);
    if !details.is_empty() {
        text.push_str(&format!("（{}）", details.join("，")));
    }
    text
}

/// 持股法人一段的各行（開頭為空行）；沒有買賣超的持股時回空陣列。
pub(super) fn lines(section: &HoldingFlowSection) -> Vec<String> {
    if section.flows.is_empty() {
        return Vec::new();
    }
    let list = |items: &mut dyn Iterator<Item = &HoldingFlow>| {
        items
            .take(TOP)
            .map(flow_text)
            .collect::<Vec<_>>()
            .join("、")
    };
    let mut lines = vec![String::new()];
    lines.push(format!(
        "持股{}近 {} 日買賣超（{}～{}）",
        section.label, section.period.days, section.period.from, section.period.data_date
    ));
    let buys = list(&mut section.flows.iter().filter(|flow| flow.net_lots > 0));
    if !buys.is_empty() {
        lines.push(format!("• 買超：{buys}"));
    }
    let sells = list(&mut section.flows.iter().rev().filter(|flow| flow.net_lots < 0));
    if !sells.is_empty() {
        lines.push(format!("• 賣超：{sells}"));
    }
    lines
}
