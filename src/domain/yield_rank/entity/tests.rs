//! 殖利率排行實體的測試。

use rust_decimal_macros::dec;

use super::*;

/// 工廠方法必須原樣保留各欄位，欄位順序寫錯會讓兩個 i64 序號互換而不易察覺。
#[test]
fn new_keeps_all_fields() {
    let date = NaiveDate::from_ymd_opt(2026, 8, 21).unwrap();
    let rank = YieldRank::new(date, "2330".to_string(), 101, 202, dec!(2.35));

    assert_eq!(rank.date, date);
    assert_eq!(rank.security_code, "2330");
    assert_eq!(rank.daily_quotes_serial, 101);
    assert_eq!(rank.dividend_serial, 202);
    assert_eq!(rank.r#yield, dec!(2.35));
}

/// 同值實體必須相等（PartialEq 供去重與測試斷言使用）。
#[test]
fn equality_compares_by_value() {
    let date = NaiveDate::from_ymd_opt(2026, 8, 21).unwrap();
    let one = YieldRank::new(date, "2330".to_string(), 101, 202, dec!(2.35));
    let another = YieldRank::new(date, "2330".to_string(), 101, 202, dec!(2.35));
    let different_yield = YieldRank::new(date, "2330".to_string(), 101, 202, dec!(2.36));

    assert_eq!(one, another);
    assert_ne!(one, different_yield);
}
