//! 通知訊息的格式化。

use super::*;

#[test]
fn formatting_helpers() {
    assert_eq!(signed_lots(9_770_520), "+9,771");
    assert_eq!(signed_lots(-2_851_652), "-2,852");
    assert_eq!(signed_lots(400), "0");
    assert_eq!(
        flow_note(9_770_520, Some(87_000_000), 3),
        "（11.2%，連 3 買）"
    );
    assert_eq!(flow_note(-500_000, None, -2), "（連 2 賣）");
    assert_eq!(flow_note(0, Some(1_000), 1), "");
    assert_eq!(balance_change(30_939, 30_135), "（-2.6%）");
    assert_eq!(balance_change(0, 10), "");
    assert_eq!(streak_key("2330"), "chip:streak:2330");
}

/// 每檔一段：股名標題、法人一行、信用一行，特殊字元依 MarkdownV2 跳脫。
#[test]
fn build_message_lists_each_reported_stock() {
    let reports = vec![report(
        Some(flow(9_770_520, 589_108, 491_655)),
        Some(MarginBalance {
            margin_previous: 30_939,
            margin_today: 30_135,
            short_previous: 18,
            short_today: 46,
        }),
        Some(87_000_000),
        Streak {
            date: day(6),
            foreign: 3,
            trust: 1,
        },
    )];
    let message = build_message(day(6), &reports, |_| "台積電".to_string());

    assert!(
        message.starts_with("持股法人與信用交易︰2026\\-10\\-06\n"),
        "{message}"
    );
    assert!(message.contains("\n2330 台積電\n"), "{message}");
    assert!(
        message.contains(
            "法人︰外資 \\+9,771（11\\.2%，連 3 買） 投信 \\+589（0\\.7%） 自營 \\+492\n"
        ),
        "{message}"
    );
    assert!(
        message.contains("信用︰融資 30,135 張（\\-2\\.6%） 融券 46 張（\\+155\\.6%）\n"),
        "{message}"
    );
}
