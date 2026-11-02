//! 每週市場摘要的「下週持股除權息」一段：下週一到週五持股（含 ETF）的除權息日、每股股利、
//! 預估入帳與近 5 年填息。
//!
//! 事件與金額來自 [`crate::app::calculation::upcoming_dividends`]（與 `/portfolio/upcoming-dividends`
//! 相同），填息文字沿用 [`FillHints::text_for`]；下週沒有就不列。

use chrono::{Datelike, Days, NaiveDate};
use rust_decimal::Decimal;

use super::super::format_decimal_with_commas;
use crate::app::calculation::{
    dividend_fill_hints::FillHints, upcoming_dividends::UpcomingDividend,
};

/// 下週持股除權息一段。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct UpcomingSection {
    /// 下週一。
    pub(super) from: NaiveDate,
    /// 下週五。
    pub(super) to: NaiveDate,
    /// 依除權息日、代號排序。
    pub(super) items: Vec<UpcomingLine>,
}

/// 一檔持股在某天的除權息與要附的填息文字。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct UpcomingLine {
    pub(super) dividend: UpcomingDividend,
    /// 近 5 年填息或同殖利率組基準（[`FillHints::text_for`]）。
    pub(super) fill: Option<String>,
}

/// 下週一到週五；週六執行時是後天到下下週一的前一天。
pub(super) fn next_week(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let days_to_monday = 7 - u64::from(today.weekday().num_days_from_monday());
    let monday = today + Days::new(days_to_monday);
    (monday, monday + Days::new(4))
}

/// 組出下週持股除權息一段，每檔附填息文字；沒有持股除權息時回 `None`。
pub(super) fn upcoming_section(
    dividends: Vec<UpcomingDividend>,
    hints: &FillHints,
    today: NaiveDate,
) -> Option<UpcomingSection> {
    if dividends.is_empty() {
        return None;
    }
    let items = dividends
        .into_iter()
        .map(|dividend| UpcomingLine {
            fill: hints.text_for(
                &dividend.stock_symbol,
                dividend.cash_dividend,
                dividend.closing_price,
            ),
            dividend,
        })
        .collect();
    let (from, to) = next_week(today);
    Some(UpcomingSection { from, to, items })
}

/// 下週持股除權息一段的各行（開頭為空行）。
pub(super) fn lines(section: &UpcomingSection) -> Vec<String> {
    let mut lines = vec![
        String::new(),
        format!(
            "下週持股除權息（{}～{}）",
            section.from.format("%m-%d"),
            section.to.format("%m-%d")
        ),
    ];
    for item in &section.items {
        let dividend = &item.dividend;
        let fill = item
            .fill
            .as_ref()
            .map_or_else(String::new, |text| format!("；{text}"));
        lines.push(format!(
            "• {} {} {}（{} 股）{}{fill}",
            dividend.ex_date.format("%m-%d"),
            dividend.stock_symbol,
            dividend.name,
            format_decimal_with_commas(Decimal::from(dividend.shares)),
            dividend_text(dividend)
        ));
    }
    let total: Decimal = section
        .items
        .iter()
        .map(|item| item.dividend.cash_amount())
        .sum();
    if section.items.len() > 1 && total > Decimal::ZERO {
        lines.push(format!(
            "• 合計預估現金入帳 {} 元（未扣二代健保與稅）",
            format_decimal_with_commas(total)
        ));
    }
    lines
}

/// 除息與除權的每股股利與預估入帳。
fn dividend_text(item: &UpcomingDividend) -> String {
    let mut parts = Vec::new();
    if item.cash_dividend > Decimal::ZERO {
        parts.push(format!(
            "除息 {} 元、預估入帳 {} 元",
            format_decimal_with_commas(item.cash_dividend),
            format_decimal_with_commas(item.cash_amount())
        ));
    }
    if item.stock_dividend > Decimal::ZERO {
        parts.push(format!(
            "除權 {} 元、預估配股 {} 股",
            format_decimal_with_commas(item.stock_dividend),
            format_decimal_with_commas(item.stock_shares())
        ));
    }
    parts.join("；")
}
