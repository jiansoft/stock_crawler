//! 外資及陸資持股領域實體的測試。

use rust_decimal_macros::dec;

use super::*;

fn alert(change_20d: Option<Decimal>) -> HoldingForeignHoldingAlert {
    HoldingForeignHoldingAlert {
        stock_symbol: "2330".to_string(),
        stock_name: "台積電".to_string(),
        date: NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
        share_holding_percentage: dec!(72.5),
        change_5d: None,
        change_20d,
        streak: 0,
    }
}

#[test]
fn significant_direction_uses_inclusive_threshold_both_ways() {
    let threshold = SIGNIFICANT_CHANGE_20D_PERCENTAGE_POINTS;
    assert_eq!(
        alert(Some(dec!(3))).significant_direction(threshold),
        Some(ForeignHoldingDirection::Increase)
    );
    assert_eq!(
        alert(Some(dec!(-3.2))).significant_direction(threshold),
        Some(ForeignHoldingDirection::Decrease)
    );
    assert_eq!(
        alert(Some(dec!(2.99))).significant_direction(threshold),
        None
    );
    assert_eq!(
        alert(Some(dec!(-2.99))).significant_direction(threshold),
        None
    );
}

/// 歷史不足 20 個交易日時沒有 20 日變化，不能當成 0 而是完全不通知。
#[test]
fn significant_direction_is_none_without_enough_history() {
    assert_eq!(
        alert(None).significant_direction(SIGNIFICANT_CHANGE_20D_PERCENTAGE_POINTS),
        None
    );
}

#[test]
fn threshold_constant_is_three_percentage_points() {
    assert_eq!(SIGNIFICANT_CHANGE_20D_PERCENTAGE_POINTS, dec!(3));
}
