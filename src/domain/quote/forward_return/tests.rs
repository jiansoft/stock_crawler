//! 未來報酬的測試。

use super::*;

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).expect("有效日期")
}

/// 第 N 個交易日：由新到舊的第 N 個（跳過假日）；結束日不是交易日或日數不足為 None。
#[test]
fn base_date_counts_trading_days_back() {
    let dates = [day(13), day(12), day(8), day(7), day(6), day(5), day(2)];
    assert_eq!(base_date(day(13), 1, &dates), Some(day(12)));
    assert_eq!(base_date(day(13), 5, &dates), Some(day(5)));
    assert_eq!(base_date(day(13), 7, &dates), None);
    assert_eq!(base_date(day(11), 1, &dates), None);
    assert_eq!(base_date(day(13), 1, &[]), None);
}
