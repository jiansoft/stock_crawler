//! 每週市場摘要的「下週持股除權息」一段：下週一到週五持股（含 ETF）的除權息日、每股股利、
//! 預估入帳與近 5 年填息。
//!
//! 每檔一行的文字與 08:00 持股除權息預告共用 [`super::super::upcoming_dividend`]；下週沒有就不列。

use chrono::{Datelike, Days, NaiveDate};

pub(super) use super::super::upcoming_dividend::UpcomingLine;
use super::super::upcoming_dividend::{item_lines, with_fill_hints};
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
    let (from, to) = next_week(today);
    Some(UpcomingSection {
        from,
        to,
        items: with_fill_hints(dividends, hints),
    })
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
    lines.extend(item_lines(&section.items));
    lines
}
