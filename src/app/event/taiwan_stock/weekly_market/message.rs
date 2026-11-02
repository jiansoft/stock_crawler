//! 把各段組成一則 MarkdownV2 訊息：標題（資料日）、各段文字、估計值與投資參考聲明。

use std::fmt::Write;

use super::{
    CrossingSection, FlowSection, HoldingSection, IndustrySection, RegimeSection, StrategySection,
    allocation, breakout, crossing, dividend, etf, flows, fundamentals, holding, holding_flow,
    industry, market, strategy, upcoming_dividend,
};
use crate::core::util::text;

/// 組出訊息；各段都沒有內容時回 `None`。
pub(super) fn build_message(
    market: Option<&RegimeSection>,
    industries: Option<&IndustrySection>,
    crossings: Option<&CrossingSection>,
    flows: &[FlowSection],
    holdings: HoldingSection<'_>,
    strategy: Option<&StrategySection>,
    stock_name: impl Fn(&str) -> String,
) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut data_date = None;

    if let Some(section) = market {
        data_date = Some(section.state.date);
        lines.extend(market::lines(section));
    }

    if let Some(section) = industries.filter(|section| !section.industries.is_empty()) {
        data_date = Some(section.data_date);
        lines.extend(industry::lines(section));
    }

    if let Some(section) = crossings {
        let rows = crossing::lines(section, &stock_name);
        if !rows.is_empty() {
            data_date.get_or_insert(section.to);
            lines.extend(rows);
        }
    }

    if let Some(section) = holdings.breakouts {
        lines.extend(breakout::lines(section));
    }

    for section in flows
        .iter()
        .filter(|section| !section.industries.is_empty())
    {
        data_date.get_or_insert(section.period.data_date);
        lines.extend(flows::lines(section));
    }

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
        data_date.get_or_insert(section.to);
        lines.extend(etf::lines(section));
    }
    lines.extend(allocation::lines(holdings.allocation));
    if let Some(section) = strategy {
        lines.extend(strategy::lines(section));
    }

    if lines.is_empty() {
        return None;
    }
    let mut message = String::with_capacity(lines.len() * 96);
    let title = match data_date {
        Some(date) => format!("每週市場摘要︰{date}"),
        None => "每週市場摘要".to_owned(),
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
