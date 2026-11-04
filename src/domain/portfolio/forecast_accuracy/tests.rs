//! 股利預估誤差比對的單元測試。

use rust_decimal_macros::dec;

use super::*;

fn date(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, month, day).expect("有效日期")
}

fn forecast(symbol: &str, pay_date: NaiveDate, amount: Decimal, basis: &str) -> ForecastedPayout {
    ForecastedPayout {
        stock_symbol: symbol.to_owned(),
        name: format!("{symbol}名"),
        pay_date,
        amount,
        basis: basis.to_owned(),
    }
}

fn actual(symbol: &str, pay_date: NaiveDate, amount: Decimal) -> ActualPayout {
    ActualPayout {
        stock_symbol: symbol.to_owned(),
        name: format!("{symbol}實"),
        pay_date,
        amount,
    }
}

/// 同月不同天也算配對；誤差以實際為分母，差額為實際 − 預估。
#[test]
fn compare_matches_by_symbol_and_pay_month() {
    let result = compare(
        &[forecast("0056", date(11, 11), dec!(2800), "announced")],
        &[actual("0056", date(11, 20), dec!(2872.4))],
        date(11, 30),
    );

    let item = &result.items[0];
    assert_eq!(item.status, AccuracyStatus::Matched);
    assert_eq!(item.month, date(11, 1));
    assert_eq!(item.name, "0056名", "名稱優先用快照");
    assert_eq!(item.actual, Some(dec!(2872.4)));
    assert_eq!(item.difference, Some(dec!(72.4)));
    assert_eq!(item.error_percent, Some(dec!(2.52)));
    assert_eq!(
        result.by_basis,
        vec![BasisAccuracy {
            basis: "announced".to_owned(),
            count: 1,
            median_error_percent: Some(dec!(2.52)),
        }]
    );
    assert_eq!(
        (result.settled_forecast, result.settled_actual),
        (dec!(2800), dec!(2872.4))
    );
}

/// 發放日還沒到的是待定、已過沒入帳的是漏領、沒預估的入帳另列；待定不計入已結算合計。
#[test]
fn compare_classifies_pending_missed_and_unforecast() {
    let result = compare(
        &[
            forecast("2330", date(12, 10), dec!(500), "latest_amount"),
            forecast("2884", date(10, 15), dec!(300), "estimated"),
        ],
        &[actual("00878", date(11, 18), dec!(120))],
        date(11, 30),
    );

    let statuses: Vec<(&str, AccuracyStatus)> = result
        .items
        .iter()
        .map(|item| (item.stock_symbol.as_str(), item.status))
        .collect();
    assert_eq!(
        statuses,
        vec![
            ("2884", AccuracyStatus::Missed),
            ("00878", AccuracyStatus::Unforecast),
            ("2330", AccuracyStatus::Pending),
        ],
        "依月份排序"
    );
    let missed = &result.items[0];
    assert_eq!(
        (missed.actual, missed.difference),
        (Some(dec!(0)), Some(dec!(-300)))
    );
    assert_eq!(missed.error_percent, None);
    let unforecast = &result.items[1];
    assert_eq!(
        (unforecast.basis.as_str(), unforecast.name.as_str()),
        ("", "00878實")
    );
    let pending = &result.items[2];
    assert_eq!((pending.actual, pending.difference), (None, None));
    assert!(result.by_basis.is_empty(), "沒有配對成功的就沒有依據彙總");
    assert_eq!(
        (result.settled_forecast, result.settled_actual),
        (dec!(300), dec!(120))
    );
}

/// 同月多筆預估（月配兩期）合併計算，依據去重排序後串接；中位數取偶數筆的平均。
#[test]
fn compare_merges_same_month_and_takes_basis_median() {
    let result = compare(
        &[
            forecast("00929", date(11, 5), dec!(60), "cadence"),
            forecast("00929", date(11, 25), dec!(40), "announced"),
            forecast("00919", date(11, 15), dec!(200), "cadence"),
            forecast("00713", date(11, 15), dec!(90), "cadence"),
        ],
        &[
            actual("00929", date(11, 6), dec!(50)),
            actual("00929", date(11, 26), dec!(50)),
            actual("00919", date(11, 16), dec!(250)),
            actual("00713", date(11, 16), dec!(100)),
            actual("00713", date(11, 17), dec!(0)),
        ],
        date(11, 30),
    );

    let merged = result
        .items
        .iter()
        .find(|item| item.stock_symbol == "00929")
        .expect("00929 有比對結果");
    assert_eq!(merged.basis, "announced/cadence");
    assert_eq!(
        (merged.forecast, merged.actual),
        (dec!(100), Some(dec!(100)))
    );
    assert_eq!(merged.error_percent, Some(dec!(0)));
    let cadence = result
        .by_basis
        .iter()
        .find(|row| row.basis == "cadence")
        .expect("cadence 有彙總");
    // 00919 誤差 20%、00713 誤差 10%，中位數 15%。
    assert_eq!(cadence.count, 2);
    assert_eq!(cadence.median_error_percent, Some(dec!(15)));
}

/// 實際入帳為 0（例如只配股）時沒有誤差百分比，也不計入依據彙總。
#[test]
fn compare_skips_error_percent_when_actual_is_zero() {
    let result = compare(
        &[forecast("2330", date(11, 10), dec!(500), "estimated")],
        &[actual("2330", date(11, 10), dec!(0))],
        date(11, 30),
    );

    assert_eq!(result.items[0].status, AccuracyStatus::Matched);
    assert_eq!(result.items[0].error_percent, None);
    assert!(result.by_basis.is_empty());
}

/// API 名稱固定為 snake_case。
#[test]
fn status_names_are_snake_case() {
    let names: Vec<&str> = [
        AccuracyStatus::Matched,
        AccuracyStatus::Pending,
        AccuracyStatus::Missed,
        AccuracyStatus::Unforecast,
    ]
    .into_iter()
    .map(AccuracyStatus::as_str)
    .collect();
    assert_eq!(names, ["matched", "pending", "missed", "unforecast"]);
}

/// 只取已入帳的現金：推算的、只配股的、沒有發放日的都排除。
#[test]
fn actual_payouts_keeps_recorded_cash_only() {
    let allocation = |symbol: &str, cash: Decimal, payable: Option<NaiveDate>, estimated: bool| {
        PayoutAllocation {
            member_id: 1,
            stock_symbol: symbol.to_owned(),
            name: symbol.to_owned(),
            quarter: "Q3".to_owned(),
            cash_dividend: dec!(1),
            stock_dividend: dec!(0),
            cash_payable: payable,
            stock_payable: None,
            cash_ex: Some(date(10, 1)),
            stock_ex: None,
            shares: 1_000,
            cash_amount: cash,
            stock_shares: dec!(0),
            estimated,
        }
    };

    let actuals = actual_payouts(&[
        allocation("0056", dec!(1000), Some(date(11, 11)), false),
        allocation("2330", dec!(500), Some(date(11, 12)), true),
        allocation("2884", dec!(0), Some(date(11, 13)), false),
        allocation("00878", dec!(300), None, false),
    ]);

    assert_eq!(
        actuals,
        vec![actual_named("0056", date(11, 11), dec!(1000))]
    );
}

fn actual_named(symbol: &str, pay_date: NaiveDate, amount: Decimal) -> ActualPayout {
    ActualPayout {
        stock_symbol: symbol.to_owned(),
        name: symbol.to_owned(),
        pay_date,
        amount,
    }
}

/// 月份彙總只算該月的已配對與漏領；待定與快照沒預估的入帳不算，其他月份也不算。
#[test]
fn month_accuracy_sums_matched_and_missed_of_the_month() {
    let result = compare(
        &[
            forecast("0056", date(11, 11), dec!(2800), "announced"),
            forecast("2330", date(11, 14), dec!(3600), "estimated"),
            forecast("2884", date(11, 20), dec!(500), "cadence"),
            forecast("00878", date(12, 18), dec!(900), "announced"),
        ],
        &[
            actual("0056", date(11, 20), dec!(2872.4)),
            actual("2330", date(11, 14), dec!(3900)),
            actual("9999", date(11, 25), dec!(100)),
            actual("0050", date(10, 22), dec!(700)),
        ],
        date(11, 30),
    );

    let november = month_accuracy(&result, date(11, 30)).expect("11 月有預估");

    assert_eq!(
        november,
        MonthAccuracy {
            matched_count: 2,
            matched_forecast: dec!(6400),
            matched_actual: dec!(6772.4),
            missed_count: 1,
            missed_forecast: dec!(500),
        }
    );
    assert_eq!(november.matched_difference(), dec!(372.4));
    assert_eq!(november.matched_difference_percent(), Some(dec!(5.5)));
    assert_eq!(
        month_accuracy(&result, date(12, 1)),
        None,
        "12 月只有待定，不算誤差"
    );
    assert_eq!(
        month_accuracy(&result, date(10, 1)),
        None,
        "10 月只有快照沒預估的入帳"
    );
}

/// 只有漏領時沒有可算的誤差百分比。
#[test]
fn month_accuracy_percent_is_none_without_matched_actual() {
    let summary = MonthAccuracy {
        missed_count: 1,
        missed_forecast: dec!(500),
        ..MonthAccuracy::default()
    };

    assert_eq!(summary.matched_difference(), Decimal::ZERO);
    assert_eq!(summary.matched_difference_percent(), None);
}
