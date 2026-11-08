//! 市值流向實體的測試。

use super::*;
use rust_decimal_macros::dec;

#[test]
fn test_money_flow_detail_is_profitable() {
    // 損益為正
    let detail_profit = MoneyFlowDetail {
        profit_and_loss: 1500.0,
        ..Default::default()
    };
    assert!(detail_profit.is_profitable());

    // 損益為負
    let detail_loss = MoneyFlowDetail {
        profit_and_loss: -500.0,
        ..Default::default()
    };
    assert!(!detail_loss.is_profitable());

    // 損益為零
    let detail_zero = MoneyFlowDetail {
        profit_and_loss: 0.0,
        ..Default::default()
    };
    assert!(!detail_zero.is_profitable());
}

#[test]
fn test_money_flow_detail_more_is_profitable() {
    // 損益為正
    let detail_profit = MoneyFlowDetailMore {
        profit_and_loss: dec!(3000),
        ..Default::default()
    };
    assert!(detail_profit.is_profitable());

    // 損益為負
    let detail_loss = MoneyFlowDetailMore {
        profit_and_loss: dec!(-200),
        ..Default::default()
    };
    assert!(!detail_loss.is_profitable());
}
