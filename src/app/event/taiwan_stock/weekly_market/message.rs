//! 把各段組成 MarkdownV2 訊息：標題（資料日）、各段文字、估計值與投資參考聲明。
//!
//! 市場段（`market.weekly`）與持股段（`holdings.weekly`）分開組成兩則：市場段的參數沒有任何持股資料，
//! 只訂市場通知的人收到的訊息不會有持股段落（會員系統計畫 7.3 混合型通知拆段）。

use std::fmt::Write;

use jiff::civil::Date;

use super::{
    CrossingSection, FlowSection, HoldingSection, IndustrySection, RegimeSection, StrategySection,
    allocation,
    breakout::{self, BreakoutSection},
    crossing, dividend,
    etf::{self, EtfWeekSection},
    flows, fundamentals, holding, holding_flow, industry, market, strategy, upcoming_dividend,
};
use crate::core::util::text;

/// 市場段的全市場附加段落：區間突破與全市場 ETF 資金流向。
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct MarketExtras<'a> {
    /// 全市場區間突破一段。
    pub(super) breakouts: Option<&'a BreakoutSection>,
    /// ETF 資金流向（只用全市場的淨流入、淨流出）。
    pub(super) etfs: Option<&'a EtfWeekSection>,
}

/// 本週資料日：大盤、產業、相對強度穿越、產業資金與 ETF 依序取第一個有資料的（產業優先於大盤）。
pub(super) fn week_data_date(
    market: Option<&RegimeSection>,
    industries: Option<&IndustrySection>,
    crossings: Option<&CrossingSection>,
    flows: &[FlowSection],
    etfs: Option<&EtfWeekSection>,
) -> Option<Date> {
    let mut data_date = market.map(|section| section.state.date);
    if let Some(section) = industries.filter(|section| !section.industries.is_empty()) {
        data_date = Some(section.data_date);
    }
    if let Some(section) = crossings
        && !crossing::lines(section, &|_: &str| String::new()).is_empty()
    {
        data_date.get_or_insert(section.to);
    }
    if let Some(section) = flows.iter().find(|section| !section.industries.is_empty()) {
        data_date.get_or_insert(section.period.data_date);
    }
    if let Some(section) = etfs {
        data_date.get_or_insert(section.to);
    }
    data_date
}

/// 組出市場段；各段都沒有內容時回 `None`。
pub(super) fn build_message(
    market: Option<&RegimeSection>,
    industries: Option<&IndustrySection>,
    crossings: Option<&CrossingSection>,
    flows: &[FlowSection],
    extras: MarketExtras<'_>,
    strategy: Option<&StrategySection>,
    stock_name: impl Fn(&str) -> String,
) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    if let Some(section) = market {
        lines.extend(market::lines(section));
    }
    if let Some(section) = industries.filter(|section| !section.industries.is_empty()) {
        lines.extend(industry::lines(section));
    }
    if let Some(section) = crossings {
        lines.extend(crossing::lines(section, &stock_name));
    }
    if let Some(section) = extras.breakouts {
        lines.extend(breakout::lines(section));
    }
    for section in flows
        .iter()
        .filter(|section| !section.industries.is_empty())
    {
        lines.extend(flows::lines(section));
    }
    if let Some(section) = extras.etfs {
        lines.extend(etf::market_lines(section));
    }
    if let Some(section) = strategy {
        lines.extend(strategy::lines(section));
    }
    let data_date = week_data_date(market, industries, crossings, flows, extras.etfs);
    finish("每週市場摘要", data_date, lines)
}

/// 組出持股段；範圍內沒有任何持股內容時回 `None`。`data_date` 為市場段的本週資料日（填息一段用）。
pub(super) fn build_holdings_message(
    data_date: Option<Date>,
    holdings: HoldingSection<'_>,
    stock_name: impl Fn(&str) -> String,
) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    lines.extend(holding::lines(holdings.moves, &stock_name));
    for section in holdings.flows {
        lines.extend(holding_flow::lines(section));
    }
    if let Some(date) = data_date {
        lines.extend(dividend::lines(holdings.fills, date, &stock_name));
    }
    if let Some(section) = holdings.upcoming {
        lines.extend(upcoming_dividend::lines(section));
    }
    lines.extend(fundamentals::lines(holdings.fundamentals, &stock_name));
    if let Some(section) = holdings.etfs {
        lines.extend(etf::held_lines(section));
    }
    lines.extend(allocation::lines(holdings.allocation));
    let data_date = data_date.or_else(|| holdings.etfs.map(|section| section.to));
    finish("每週持股摘要", data_date, lines)
}

/// 標題（資料日）、各行與兩行聲明；沒有任何行時回 `None`。
fn finish(title: &str, data_date: Option<Date>, lines: Vec<String>) -> Option<String> {
    if lines.is_empty() {
        return None;
    }
    let mut message = String::with_capacity(lines.len() * 96);
    let title = match data_date {
        Some(date) => format!("{title}︰{date}"),
        None => title.to_owned(),
    };
    let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(title));
    for line in lines {
        let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(line));
    }
    let _ = writeln!(
        &mut message,
        "\n{}",
        text::escape_markdown_v2("＊金額為每日淨股數×當日收盤價的估計值，過去表現不代表未來結果")
    );
    // 策略段也屬投資參考輸出，與 Data API／MCP 使用相同的完整聲明。
    let _ = writeln!(
        &mut message,
        "{}",
        text::escape_markdown_v2("僅供參考，不構成投資建議，亦不會自動下單")
    );
    Some(message)
}
