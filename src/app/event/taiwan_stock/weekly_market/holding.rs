//! 每週市場摘要的持股相對強度一段：持股 RS 與一週前相比上升、下降最多的幾檔。

use super::{
    TOP, WEEK_DAYS, dividend::HoldingFill, etf::EtfWeekSection, holding_flow::HoldingFlowSection,
};
use crate::domain::{
    portfolio::health::HoldingFundamentals, quote::relative_strength::RelativeStrengthDay,
};

/// 持股相關的各段：RS 一週變化、法人與主力一週買賣超、填息、基本面警示，以及 ETF 資金流向。
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct HoldingSection<'a> {
    /// 持股 RS 一週變化。
    pub(super) moves: &'a [HoldingMove],
    /// 外資、投信與主力在持股的一週買賣超。
    pub(super) flows: &'a [HoldingFlowSection],
    /// 持股近一年每次除權息的填息結果。
    pub(super) fills: &'a [HoldingFill],
    /// 持股的營收與三率（基本面警示一段用）。
    pub(super) fundamentals: &'a [HoldingFundamentals],
    /// 全市場與持有 ETF 的一週淨申購與折溢價。
    pub(super) etfs: Option<&'a EtfWeekSection>,
}

/// 一檔持股的 RS 一週變化。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct HoldingMove {
    pub(super) stock_symbol: String,
    pub(super) from: u8,
    pub(super) to: u8,
}

impl HoldingMove {
    pub(super) fn change(&self) -> i16 {
        i16::from(self.to) - i16::from(self.from)
    }
}

/// 由依代號、再由新到舊排列的分數算出每檔持股最新分數與最舊一筆的差；只有一筆的略過。
pub(super) fn holding_moves(days: &[RelativeStrengthDay]) -> Vec<HoldingMove> {
    days.chunk_by(|a, b| a.stock_symbol == b.stock_symbol)
        .filter_map(|group| {
            let (latest, oldest) = (group.first()?, group.last()?);
            (group.len() > 1).then(|| HoldingMove {
                stock_symbol: latest.stock_symbol.clone(),
                from: oldest.rs_score,
                to: latest.rs_score,
            })
        })
        .collect()
}

/// 持股相對強度一段的各行（開頭為空行）；沒有分數變化時回空陣列。
pub(super) fn lines(moves: &[HoldingMove], stock_name: &impl Fn(&str) -> String) -> Vec<String> {
    let mut sorted: Vec<&HoldingMove> = moves.iter().filter(|item| item.change() != 0).collect();
    sorted.sort_by(|a, b| {
        b.change()
            .cmp(&a.change())
            .then_with(|| a.stock_symbol.cmp(&b.stock_symbol))
    });
    if sorted.is_empty() {
        return Vec::new();
    }
    let line = |item: &&HoldingMove| {
        format!(
            "{} {} {} → {}（{:+}）",
            item.stock_symbol,
            stock_name(&item.stock_symbol),
            item.from,
            item.to,
            item.change()
        )
    };
    let mut lines = vec![String::new()];
    lines.push(format!("持股相對強度（與 {WEEK_DAYS} 個交易日前相比）"));
    let up: Vec<String> = sorted
        .iter()
        .filter(|item| item.change() > 0)
        .take(TOP)
        .map(line)
        .collect();
    if !up.is_empty() {
        lines.push(format!("• 上升：{}", up.join("、")));
    }
    let down: Vec<String> = sorted
        .iter()
        .rev()
        .filter(|item| item.change() < 0)
        .take(TOP)
        .map(line)
        .collect();
    if !down.is_empty() {
        lines.push(format!("• 下降：{}", down.join("、")));
    }
    lines
}
