//! `months_between` 的月份區間計算。

use super::*;

#[test]
fn months_between_is_inclusive_on_both_ends() {
    let months = months_between(date(2021, 11, 15), date(2022, 2, 3)).expect("應成功");

    assert_eq!(
        months,
        vec![
            date(2021, 11, 1),
            date(2021, 12, 1),
            date(2022, 1, 1),
            date(2022, 2, 1),
        ]
    );
}

#[test]
fn months_between_handles_a_single_month() {
    let months = months_between(date(2021, 8, 2), date(2021, 8, 31)).expect("應成功");
    assert_eq!(months, vec![date(2021, 8, 1)]);
}

#[test]
fn months_between_spans_seven_years_of_the_known_gap() {
    // 2015-01 ~ 2021-12 共 84 個月，這正是 ETF 缺口的長度。
    let months = months_between(date(2015, 1, 1), date(2021, 12, 31)).expect("應成功");
    assert_eq!(months.len(), 84);
    assert_eq!(months.first(), Some(&date(2015, 1, 1)));
    assert_eq!(months.last(), Some(&date(2021, 12, 1)));
}

#[test]
fn months_between_rejects_a_reversed_range() {
    assert!(months_between(date(2022, 1, 1), date(2021, 12, 1)).is_err());
}
