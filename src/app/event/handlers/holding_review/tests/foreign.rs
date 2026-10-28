//! 持股外資持股變化通知的訊息與去重鍵測試。

use super::*;

#[test]
fn foreign_holding_message_groups_by_direction_and_sorts_by_magnitude() {
    let big_up = foreign_alert("6488", "環球晶", Some(dec!(5.2)), 4);
    let small_up = foreign_alert("3105", "穩懋", Some(dec!(3.1)), 1);
    let down = foreign_alert("8069", "元太", Some(dec!(-3.5)), -3);
    let date = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();

    let msg = EventDispatcher::build_foreign_holding_message(date, &[&small_up, &big_up], &[&down]);

    let increase_at = msg.find("外資增持").unwrap();
    let decrease_at = msg.find("外資減持").unwrap();
    assert!(increase_at < decrease_at);
    // 增持依 20 日變化由大到小：環球晶在穩懋前面
    assert!(msg.find("環球晶").unwrap() < msg.find("穩懋").unwrap());
    assert!(msg.contains("20日︰\\+5\\.2"));
    assert!(msg.contains("持股︰26\\.37%"));
    assert!(msg.contains("連續增持 4 日"));
    assert!(msg.contains("連續減持 3 日"));
    // 連續 1 日不註明
    assert_eq!(msg.matches("連續增持").count(), 1);
}

#[test]
fn foreign_holding_message_omits_empty_group() {
    let down = foreign_alert("8069", "元太", Some(dec!(-3.5)), -1);
    let msg = EventDispatcher::build_foreign_holding_message(
        NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
        &[],
        &[&down],
    );

    assert!(!msg.contains("外資增持"));
    assert!(msg.contains("外資減持"));
}

#[test]
fn format_optional_change_shows_dash_without_history() {
    assert_eq!(EventDispatcher::format_optional_change(None), "—");
    assert_eq!(
        EventDispatcher::format_optional_change(Some(dec!(1.2))),
        "+1.2"
    );
    assert_eq!(
        EventDispatcher::format_optional_change(Some(dec!(-0.5))),
        "-0.5"
    );
}

#[test]
fn foreign_holding_key_separates_direction_and_symbol() {
    assert_eq!(
        EventDispatcher::foreign_holding_key(ForeignHoldingDirection::Increase, "2330"),
        "holding_review:qfii:up:2330"
    );
    assert_eq!(
        EventDispatcher::foreign_holding_key(ForeignHoldingDirection::Decrease, "2330"),
        "holding_review:qfii:down:2330"
    );
}
