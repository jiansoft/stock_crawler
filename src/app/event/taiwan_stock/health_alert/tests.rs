//! 持股財務體質變化通知的測試。

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
