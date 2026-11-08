//! # 持股財務體質變化通知
//!
//! 07:10 重算全市場財務體質（[`crate::app::calculation::financial_health`]）後，比對持股普通股
//! 這次與上一次的 Piotroski F 分數，把值得留意的變化彙整成一則 Telegram：
//!
//! - **新年報**：評分年度往後推（新的年度財報出來），一律通知新分數與前一年度分數。
//! - **下滑**：同一評分年度分數下滑 [`DROP_ALERT_POINTS`] 分以上（來源修正財報數字）。
//! - **轉弱**：同一評分年度分數由高於 [`WEAK_SCORE`] 分落到 [`WEAK_SCORE`] 分以下。
//!
//! 比較基準就是重算前表裡的上一次結果，因此同一個變化只會通知一次；表是空的（第一次重算）
//! 時沒有基準，不通知。分數上升、沒有評分年度的股票都不通知。

use std::{collections::BTreeSet, fmt::Write};

use crate::{core::util::text, domain::financial::health::FinancialHealthSnapshot};

/// 同一評分年度下滑幾分以上要通知。
pub const DROP_ALERT_POINTS: u8 = 2;

/// 分數落到這個值以下（含）視為體質偏弱。
pub const WEAK_SCORE: u8 = 3;

/// 變化類型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChangeKind {
    /// 新的年度財報出來，評分年度往後推。
    NewYear,
    /// 同一評分年度分數下滑。
    Dropped,
    /// 同一評分年度落到偏弱區間。
    Weakened,
}

/// 一檔持股的分數變化。
#[derive(Debug, Clone, PartialEq, Eq)]
struct HealthChange {
    stock_symbol: String,
    kind: ChangeKind,
    year: i32,
    score: u8,
    evaluated: u8,
    previous_year: i32,
    previous_score: u8,
    previous_evaluated: u8,
}

/// 比對持股這次與上一次的分數，找出要通知的變化；依股票代號排序。
fn detect_changes(
    previous: &[FinancialHealthSnapshot],
    current: &[FinancialHealthSnapshot],
    holdings: &BTreeSet<String>,
) -> Vec<HealthChange> {
    let mut changes = Vec::new();
    for now in current
        .iter()
        .filter(|s| holdings.contains(&s.stock_symbol))
    {
        let Some(before) = previous.iter().find(|s| s.stock_symbol == now.stock_symbol) else {
            continue;
        };
        let (Some(year), Some(score), Some(previous_year), Some(previous_score)) = (
            now.f_score_year,
            now.f_score,
            before.f_score_year,
            before.f_score,
        ) else {
            continue;
        };
        let kind = if year > previous_year {
            ChangeKind::NewYear
        } else if year < previous_year {
            // 評分年度倒退只會是資料被刪，不是值得通知的體質變化。
            continue;
        } else if previous_score.saturating_sub(score) >= DROP_ALERT_POINTS {
            ChangeKind::Dropped
        } else if score <= WEAK_SCORE && previous_score > WEAK_SCORE {
            ChangeKind::Weakened
        } else {
            continue;
        };
        changes.push(HealthChange {
            stock_symbol: now.stock_symbol.clone(),
            kind,
            year,
            score,
            evaluated: now.f_score_evaluated,
            previous_year,
            previous_score,
            previous_evaluated: before.f_score_evaluated,
        });
    }
    changes.sort_by(|a, b| a.stock_symbol.cmp(&b.stock_symbol));
    changes
}

/// 組出通知訊息（Telegram MarkdownV2）；沒有變化時回 `None`。
///
/// 比較基準為空（第一次重算）時不通知。`stock_name` 由呼叫端提供（正式流程讀股票主檔快取）。
pub(crate) fn build_notification(
    previous: &[FinancialHealthSnapshot],
    current: &[FinancialHealthSnapshot],
    holdings: &BTreeSet<String>,
    stock_name: impl Fn(&str) -> String,
) -> Option<String> {
    if previous.is_empty() {
        return None;
    }
    let changes = detect_changes(previous, current, holdings);
    if changes.is_empty() {
        return None;
    }

    let mut message = String::with_capacity(128 + changes.len() * 128);
    let _ = writeln!(
        &mut message,
        "{}",
        text::escape_markdown_v2("持股財務體質變化（Piotroski F 分數，滿分 9）")
    );
    for change in &changes {
        let _ = writeln!(
            &mut message,
            "{}",
            text::escape_markdown_v2(describe(change, &stock_name(&change.stock_symbol)))
        );
    }
    Some(message.trim_end().to_string())
}

/// 單檔的一行說明。
fn describe(change: &HealthChange, name: &str) -> String {
    let reason = match change.kind {
        ChangeKind::NewYear => "新年報".to_string(),
        ChangeKind::Dropped => format!(
            "下滑 {} 分",
            change.previous_score.saturating_sub(change.score)
        ),
        ChangeKind::Weakened => format!("落到 {WEAK_SCORE} 分以下"),
    };
    let weak = if change.kind != ChangeKind::Weakened && change.score <= WEAK_SCORE {
        "，體質偏弱"
    } else {
        ""
    };
    format!(
        "【{} {}】{} 年 {} 分（{} 項可判定），前次 {} 年 {} 分（{} 項）：{reason}{weak}",
        change.stock_symbol,
        name,
        change.year,
        change.score,
        change.evaluated,
        change.previous_year,
        change.previous_score,
        change.previous_evaluated
    )
}

#[cfg(test)]
mod tests;
