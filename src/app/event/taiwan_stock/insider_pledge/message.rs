//! 董監質押變動的通知文字：依股票分段，逐筆描述質押、關係人質押、持股與目前狀態。

use std::fmt::Write;

use chrono::Datelike;
use rust_decimal::Decimal;

use super::super::{add_thousand_separators, format_decimal_with_commas};
use super::{Change, Insider, Report};
use crate::core::util::text;

/// 組出通知訊息（Telegram MarkdownV2）。
///
/// `stock_name` 由呼叫端提供（正式流程讀股票主檔快取），測試時可換成固定對照。
pub(super) fn build_message(reports: &[Report], stock_name: impl Fn(&str) -> String) -> String {
    let month = reports
        .iter()
        .map(|report| report.month)
        .max()
        .unwrap_or_default();
    let mut message = String::with_capacity(256 + reports.len() * 256);
    let _ = writeln!(
        &mut message,
        "{}",
        text::escape_markdown_v2(format!(
            "持股董監質押變動︰{} 年 {} 月",
            month.year(),
            month.month()
        ))
    );
    for report in reports {
        let mut heading = format!(
            "【{} {}】",
            report.stock_symbol,
            stock_name(&report.stock_symbol)
        );
        if report.baseline {
            heading.push_str("首次追蹤，目前設質");
        }
        let _ = write!(&mut message, "\n{}\n", text::escape_markdown_v2(heading));
        for change in &report.changes {
            let _ = writeln!(
                &mut message,
                "{}",
                text::escape_markdown_v2(describe(change))
            );
        }
    }
    message
}

/// 一項變動的文字。
pub(super) fn describe(change: &Change) -> String {
    match change {
        Change::Pledged { insider, before } => format!(
            "{} 設質 {} → {}{}",
            who(insider),
            lots(*before),
            lots(insider.pledged),
            pledge_ratio(insider)
        ),
        Change::RelatedPledged { insider, before } => format!(
            "{} 關係人設質 {} → {}",
            who(insider),
            lots(*before),
            lots(insider.related_pledged)
        ),
        Change::Shares { insider, before } => format!(
            "{} 持股 {} → {}",
            who(insider),
            lots(*before),
            lots(insider.shares)
        ),
        Change::Current(insider) => {
            let mut parts = Vec::new();
            if insider.pledged > 0 {
                parts.push(format!(
                    "設質 {}{}",
                    lots(insider.pledged),
                    pledge_ratio(insider)
                ));
            }
            if insider.related_pledged > 0 {
                parts.push(format!("關係人設質 {}", lots(insider.related_pledged)));
            }
            format!("{} {}", who(insider), parts.join("、"))
        }
    }
}

/// 「姓名（職稱、職稱）」。
pub(super) fn who(insider: &Insider) -> String {
    format!("{}（{}）", insider.name, insider.titles.join("、"))
}

/// 設質佔持股比例，例如「（佔持股 77.4%）」；沒有持股時不顯示。
pub(super) fn pledge_ratio(insider: &Insider) -> String {
    if insider.shares <= 0 || insider.pledged <= 0 {
        return String::new();
    }
    let ratio =
        Decimal::from(insider.pledged) * Decimal::ONE_HUNDRED / Decimal::from(insider.shares);
    format!("（佔持股 {}%）", format_decimal_with_commas(ratio))
}

/// 股數轉成張（四捨五入）；不足一張時顯示股數。
pub(super) fn lots(shares: i64) -> String {
    if shares != 0 && shares.abs() < 1_000 {
        return format!("{shares} 股");
    }
    let lots = (shares + shares.signum() * 500) / 1_000;
    format!("{} 張", add_thousand_separators(&lots.to_string()))
}
