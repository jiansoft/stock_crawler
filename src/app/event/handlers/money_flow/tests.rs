//! `MoneyFlowRecalculated` 事件處理：重新計算並發送 Telegram 市值變化通知的測試。

use super::*;

#[test]
fn test_build_money_change_message_includes_hugo() {
    use crate::domain::money_flow::entity::MoneyFlowMemberWithPreviousDay;
    use chrono::NaiveDate;
    use rust_decimal_macros::dec;

    let date = NaiveDate::parse_from_str("2026-04-02", "%Y-%m-%d").unwrap();
    let previous_date = NaiveDate::parse_from_str("2026-04-01", "%Y-%m-%d").unwrap();
    let rows = vec![
        MoneyFlowMemberWithPreviousDay {
            date,
            previous_date: Some(previous_date),
            member_id: 0,
            market_value: dec!(4273187.20),
            previous_market_value: dec!(4053774.55),
        },
        MoneyFlowMemberWithPreviousDay {
            date,
            previous_date: Some(previous_date),
            member_id: 1,
            market_value: dec!(2195395.10),
            previous_market_value: dec!(2207807.70),
        },
        MoneyFlowMemberWithPreviousDay {
            date,
            previous_date: Some(previous_date),
            member_id: 2,
            market_value: dec!(1500000.00),
            previous_market_value: dec!(1400000.00),
        },
        MoneyFlowMemberWithPreviousDay {
            date,
            previous_date: Some(previous_date),
            member_id: 3,
            market_value: dec!(577792.10),
            previous_market_value: dec!(445966.85),
        },
    ];

    let msg = EventDispatcher::build_money_change_message(&rows).expect("message should be built");

    assert!(msg.contains("合計"));
    assert!(msg.contains("Eddie"));
    assert!(msg.contains("Unice"));
    assert!(msg.contains("Hugo"));
    assert!(msg.contains("4,273,187\\.20"));
    assert!(msg.contains("577,792\\.10"));
    assert!(msg.contains("\\-12,412\\.60"));
}
