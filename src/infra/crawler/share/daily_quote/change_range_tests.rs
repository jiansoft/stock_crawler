//! 漲跌幅（參考價）計算的單元測試。

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::change_range_percent;

/// 一般交易日：參考價就是前一日收盤（00715L 2026-10-02 收 70.75、漲 7.2）。
#[test]
fn uses_the_reference_price_implied_by_the_change() {
    let range = change_range_percent(dec!(70.75), dec!(7.2));
    assert_eq!(range.round_dp(4), dec!(11.3297));
    let range = change_range_percent(dec!(2500), dec!(-10));
    assert_eq!(range.round_dp(4), dec!(-0.3984));
}

/// 參考價不等於前一個成交日收盤時，仍以參考價計算，正負號與漲跌一致
/// （6661 威健生技 2026-10-05：前一個成交日收 16.20，參考價 16.75，收 16.60、漲跌 -0.15）。
#[test]
fn follows_the_exchange_reference_price() {
    let range = change_range_percent(dec!(16.60), dec!(-0.15));
    assert_eq!(range.round_dp(4), dec!(-0.8955));
    assert!(range.is_sign_negative());
}

#[test]
fn zero_change_or_non_positive_reference_is_zero() {
    assert_eq!(
        change_range_percent(dec!(2500), Decimal::ZERO),
        Decimal::ZERO
    );
    // 推不出正的參考價時為 0，不可除以零。
    assert_eq!(change_range_percent(dec!(1), dec!(1)), Decimal::ZERO);
    assert_eq!(change_range_percent(dec!(1), dec!(2)), Decimal::ZERO);
}
