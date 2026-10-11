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
mod tests {
    use super::*;

    fn snapshot(symbol: &str, year: Option<i32>, score: Option<u8>) -> FinancialHealthSnapshot {
        FinancialHealthSnapshot {
            stock_symbol: symbol.to_owned(),
            f_score_year: year,
            f_score: score,
            f_score_evaluated: if score.is_some() { 9 } else { 0 },
            quarter_year: None,
            quarter: None,
            gross_margin_percent: None,
            debt_ratio_percent: None,
            free_cash_flow_ttm: None,
        }
    }

    fn holdings(symbols: &[&str]) -> BTreeSet<String> {
        symbols.iter().map(|s| (*s).to_owned()).collect()
    }

    fn kinds(changes: &[HealthChange]) -> Vec<(&str, ChangeKind)> {
        changes
            .iter()
            .map(|c| (c.stock_symbol.as_str(), c.kind))
            .collect()
    }

    /// 新年報一律通知；同年度下滑 2 分以上或落到 3 分以下才通知；上升、小幅下滑、
    /// 非持股、缺分數、沒有前次資料與評分年度倒退都不通知。
    #[test]
    fn detect_changes_applies_each_rule() {
        let previous = [
            snapshot("1101", Some(2024), Some(9)),
            snapshot("2317", Some(2025), Some(6)),
            snapshot("2330", Some(2025), Some(4)),
            snapshot("2412", Some(2025), Some(5)),
            snapshot("2454", Some(2025), Some(5)),
            snapshot("2603", Some(2025), Some(7)),
            snapshot("2882", Some(2025), None),
            snapshot("3008", Some(2025), Some(6)),
            snapshot("9999", Some(2025), Some(9)),
        ];
        let current = [
            snapshot("1101", Some(2025), Some(9)), // 新年報（分數沒變也通知）
            snapshot("2317", Some(2025), Some(4)), // 下滑 2 分
            snapshot("2330", Some(2025), Some(3)), // 落到 3 分以下
            snapshot("2412", Some(2025), Some(4)), // 只下滑 1 分
            snapshot("2454", Some(2025), Some(7)), // 上升
            snapshot("2603", Some(2024), Some(1)), // 評分年度倒退
            snapshot("2882", Some(2025), Some(4)), // 前次沒有分數
            snapshot("3008", Some(2025), Some(1)), // 非持股
            snapshot("5880", Some(2025), Some(1)), // 沒有前次資料
            snapshot("9999", Some(2025), Some(1)), // 非持股
        ];
        let held = holdings(&[
            "1101", "2317", "2330", "2412", "2454", "2603", "2882", "5880",
        ]);

        let changes = detect_changes(&previous, &current, &held);
        assert_eq!(
            kinds(&changes),
            [
                ("1101", ChangeKind::NewYear),
                ("2317", ChangeKind::Dropped),
                ("2330", ChangeKind::Weakened),
            ]
        );
        assert_eq!(
            (changes[0].previous_year, changes[0].previous_score),
            (2024, 9)
        );
    }

    /// 同年度一次下滑很多且落到偏弱區間時記為「下滑」，說明文字補上體質偏弱。
    #[test]
    fn describe_marks_weak_scores() {
        let change = HealthChange {
            stock_symbol: "2317".to_owned(),
            kind: ChangeKind::Dropped,
            year: 2025,
            score: 2,
            evaluated: 9,
            previous_year: 2025,
            previous_score: 6,
            previous_evaluated: 9,
        };
        assert_eq!(
            describe(&change, "鴻海"),
            "【2317 鴻海】2025 年 2 分（9 項可判定），前次 2025 年 6 分（9 項）：下滑 4 分，體質偏弱"
        );
        let weakened = HealthChange {
            kind: ChangeKind::Weakened,
            score: 3,
            previous_score: 4,
            ..change.clone()
        };
        assert!(describe(&weakened, "鴻海").ends_with("：落到 3 分以下"));
        let new_year = HealthChange {
            kind: ChangeKind::NewYear,
            year: 2026,
            score: 8,
            ..change
        };
        assert!(describe(&new_year, "鴻海").ends_with("：新年報"));
    }

    /// 有變化才組訊息，標題與每檔一行都經過 MarkdownV2 跳脫；沒有基準或沒有變化時不發。
    #[test]
    fn build_notification_only_when_something_changed() {
        let previous = [snapshot("2330", Some(2024), Some(9))];
        let current = [snapshot("2330", Some(2025), Some(7))];
        let held = holdings(&["2330"]);
        let name = |_: &str| "台積電".to_owned();

        let message = build_notification(&previous, &current, &held, name).expect("應有通知");
        assert_eq!(
            message,
            "持股財務體質變化（Piotroski F 分數，滿分 9）\n【2330 台積電】2025 年 7 分（9 項可判定），前次 2024 年 9 分（9 項）：新年報"
        );

        assert_eq!(build_notification(&[], &current, &held, name), None);
        assert_eq!(build_notification(&current, &current, &held, name), None);
    }
}
