//! 持股即將除權息的通知文字：每檔一行（除權息日、代號名稱、股數、每股股利與預估入帳、近 5 年填息），
//! 多檔時附合計預估現金入帳。
//!
//! 週六每週市場摘要的「下週持股除權息」與 08:00 的持股除權息預告（[`super::ex_dividend`]）共用。
//! 事件與金額來自 [`crate::app::calculation::upcoming_dividends`]（與 `/portfolio/upcoming-dividends`
//! 相同），填息文字沿用 [`FillHints::text_for`]。

use rust_decimal::Decimal;

use super::format_decimal_with_commas;
use crate::app::calculation::{
    dividend_fill_hints::FillHints, upcoming_dividends::UpcomingDividend,
};

/// 一檔持股在某天的除權息與要附的填息文字。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct UpcomingLine {
    pub(super) dividend: UpcomingDividend,
    /// 近 5 年填息或同殖利率組基準（[`FillHints::text_for`]）。
    pub(super) fill: Option<String>,
}

/// 每檔附上填息文字，順序不變。
pub(super) fn with_fill_hints(
    dividends: Vec<UpcomingDividend>,
    hints: &FillHints,
) -> Vec<UpcomingLine> {
    dividends
        .into_iter()
        .map(|dividend| UpcomingLine {
            fill: hints.text_for(
                &dividend.stock_symbol,
                dividend.cash_dividend,
                dividend.closing_price,
            ),
            dividend,
        })
        .collect()
}

/// 每檔一行；多檔且有現金股利時再加一行合計預估現金入帳。
pub(super) fn item_lines(items: &[UpcomingLine]) -> Vec<String> {
    let mut lines = Vec::with_capacity(items.len() + 1);
    for item in items {
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
    let total: Decimal = items.iter().map(|item| item.dividend.cash_amount()).sum();
    if items.len() > 1 && total > Decimal::ZERO {
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
