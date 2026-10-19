//! 換股通知的訊息內容：新買進／續抱／賣出、現金與 0050 核心部位，以及對照實際持股。

use std::collections::BTreeSet;
use std::fmt::Write;

use crate::{
    app::ports::{StrategyHolding, StrategyRebalance},
    core::util::text,
};

/// 代號與名稱；沒有名稱時只有代號。
fn symbol_text(holding: &StrategyHolding) -> String {
    match holding.name.as_deref() {
        Some(name) if !name.is_empty() => format!("{} {name}", holding.symbol),
        _ => holding.symbol.clone(),
    }
}

/// 對照實際持股的幾行：策略持股中已持有與尚未持有的股票、策略賣出但仍持有的股票，以及有 0050
/// 核心部位時是否持有 0050。
fn holdings_lines(rebalance: &StrategyRebalance, held: &BTreeSet<String>) -> Vec<String> {
    let strategy: Vec<&StrategyHolding> = rebalance.added.iter().chain(&rebalance.kept).collect();
    let (owned, missing): (Vec<&StrategyHolding>, Vec<&StrategyHolding>) = strategy
        .iter()
        .partition(|holding| held.contains(&holding.symbol));
    let list = |items: &[&StrategyHolding]| {
        items
            .iter()
            .map(|holding| symbol_text(holding))
            .collect::<Vec<_>>()
            .join("、")
    };
    let mut lines = Vec::new();
    if !strategy.is_empty() {
        let mut line = format!(
            "• 對照實際持股：策略 {} 檔中已持有 {} 檔",
            strategy.len(),
            owned.len()
        );
        if !missing.is_empty() {
            let _ = write!(&mut line, "；尚未持有：{}", list(&missing));
        }
        lines.push(line);
    }
    let still_held: Vec<&StrategyHolding> = rebalance
        .exited
        .iter()
        .filter(|holding| held.contains(&holding.symbol))
        .collect();
    if !still_held.is_empty() {
        lines.push(format!("• 策略賣出但仍持有：{}", list(&still_held)));
    }
    if rebalance.etf_weight > 0 {
        let status = if held.contains("0050") {
            "已持有"
        } else {
            "尚未持有"
        };
        lines.push(format!("• 0050 核心部位：{status}"));
    }
    lines
}

/// 一檔股票：代號、名稱與權重。
fn holding_text(holding: &StrategyHolding) -> String {
    let name = holding
        .name
        .as_deref()
        .map(|name| format!(" {name}"))
        .unwrap_or_default();
    format!("{}{name} {:.1}%", holding.symbol, holding.weight)
}

/// 一類股票的一行；沒有股票時回 `None`。
fn group_line(label: &str, holdings: &[StrategyHolding]) -> Option<String> {
    if holdings.is_empty() {
        return None;
    }
    let list: Vec<String> = holdings.iter().map(holding_text).collect();
    Some(format!(
        "• {label}（{} 檔）：{}",
        holdings.len(),
        list.join("、")
    ))
}

/// 組出換股通知（MarkdownV2 已跳脫）；`actual` 為目前的實際持股代號，`None` 時不對照。
pub(super) fn build_message(
    rebalance: &StrategyRebalance,
    name: &str,
    actual: Option<&BTreeSet<String>>,
) -> String {
    let title = if name.is_empty() {
        format!("策略換股︰{}", rebalance.date)
    } else {
        format!("策略換股（{name}）︰{}", rebalance.date)
    };
    let held = rebalance.added.len() + rebalance.kept.len();
    let mut lines = Vec::new();
    if rebalance.market_filtered {
        lines.push("• 大盤濾網沒過，策略部位全部轉為現金".to_owned());
    } else if held == 0 {
        lines.push("• 沒有符合條件的股票，策略部位全部轉為現金".to_owned());
    } else if rebalance.cash > 0.0 {
        lines.push(format!(
            "• 持有 {held} 檔，策略部位現金 {:.1}%（單檔權重上限）",
            rebalance.cash
        ));
    } else {
        lines.push(format!("• 持有 {held} 檔"));
    }
    lines.extend(group_line("新買進", &rebalance.added));
    lines.extend(group_line("續抱", &rebalance.kept));
    lines.extend(group_line("賣出", &rebalance.exited));
    let mut summary = format!("• 換手 {:.1}%", rebalance.turnover * 100.0);
    if rebalance.etf_weight > 0 {
        let _ = write!(
            &mut summary,
            "；0050 核心部位 {}%（權重為策略部位內的比例，0050 照常持有）",
            rebalance.etf_weight
        );
    }
    lines.push(summary);
    lines.push(format!("• 下次換股：{} 個交易日後", rebalance.hold_days));
    if let Some(actual) = actual {
        lines.extend(holdings_lines(rebalance, actual));
    }

    let mut message = String::with_capacity(512);
    let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(title));
    for line in lines {
        let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(line));
    }
    let _ = writeln!(
        &mut message,
        "\n{}",
        text::escape_markdown_v2(
            "＊依回測條件以今天收盤選出，實際下單在下一個交易日；過去表現不代表未來結果"
        )
    );
    message
}
