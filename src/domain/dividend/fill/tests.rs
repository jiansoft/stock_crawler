//! 填息（填權）分析：除權息後股價回到除權息前一日收盤價花了幾個交易日的測試。

use rust_decimal_macros::dec;

use super::*;

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, d).expect("日期應合法")
}

fn closes(prices: &[(u32, Decimal)]) -> Vec<DailyClose> {
    prices
        .iter()
        .map(|&(d, close)| DailyClose {
            date: day(d),
            close,
        })
        .collect()
}

fn event(ex_day: u32) -> ExRightEvent {
    ExRightEvent {
        ex_date: day(ex_day),
        cash_dividend: dec!(5),
        stock_dividend: Decimal::ZERO,
    }
}

/// 第 3 個交易日收盤回到除息前一日收盤即填息；停牌（收盤 0）的日子不算交易日。
#[test]
fn fill_outcome_counts_trading_days_from_the_ex_date() {
    let prices = closes(&[
        (14, dec!(98)),
        (15, dec!(100)),
        (16, dec!(95)),
        (17, Decimal::ZERO),
        (18, dec!(99)),
        (21, dec!(100)),
        (22, dec!(97)),
    ]);
    let outcome = fill_outcome(event(16), &prices).expect("應算得出結果");
    assert_eq!(
        (outcome.pre_ex_date, outcome.pre_ex_close),
        (day(15), dec!(100))
    );
    assert_eq!(outcome.fill_date, Some(day(21)));
    assert_eq!(outcome.trading_days_to_fill, Some(3));
    assert_eq!(outcome.trading_days_since_ex, 4);
    assert_eq!(
        (outcome.latest_date, outcome.latest_close),
        (day(22), dec!(97))
    );
    assert_eq!(outcome.gap_percent(), None, "已填息不算差距");
}

/// 除息當日就回到前一日收盤算第 1 天；還沒填息時算出距離填息價的百分比。
#[test]
fn fill_outcome_handles_same_day_fill_and_unfilled_gap() {
    let same_day = closes(&[(15, dec!(100)), (16, dec!(100.5))]);
    let outcome = fill_outcome(event(16), &same_day).expect("應算得出結果");
    assert_eq!(outcome.trading_days_to_fill, Some(1));

    let unfilled = closes(&[(15, dec!(80)), (16, dec!(75)), (17, dec!(76))]);
    let outcome = fill_outcome(event(16), &unfilled).expect("應算得出結果");
    assert_eq!(outcome.fill_date, None);
    assert_eq!(outcome.trading_days_since_ex, 2);
    assert_eq!(outcome.gap_percent(), Some(dec!(5)));
}

/// 沒有除權息前的收盤，或除權息後還沒有收盤時算不出結果。
#[test]
fn fill_outcome_needs_closes_on_both_sides() {
    assert_eq!(fill_outcome(event(16), &closes(&[(16, dec!(90))])), None);
    assert_eq!(fill_outcome(event(16), &closes(&[(15, dec!(90))])), None);
    assert_eq!(
        fill_outcome(event(16), &closes(&[(15, Decimal::ZERO), (16, dec!(90))])),
        None,
        "除權息前只有停牌日"
    );
}

fn stock_event(ex_day: u32, cash: Decimal, stock: Decimal) -> ExRightEvent {
    ExRightEvent {
        ex_date: day(ex_day),
        cash_dividend: cash,
        stock_dividend: stock,
    }
}

/// 目標價只扣配股稀釋：純現金不變、配股 2 元除以 1.2、負值視為沒有配股。
#[test]
fn fill_target_adjusts_for_stock_dividend_only() {
    assert_eq!(fill_target(dec!(100), Decimal::ZERO), dec!(100));
    assert_eq!(fill_target(dec!(60), dec!(2)), dec!(50));
    assert_eq!(fill_target(dec!(100), dec!(0.5)), dec!(95.24));
    assert_eq!(fill_target(dec!(100), dec!(-1)), dec!(100));
}

/// 純配股：回到 100 ÷ 1.2 就填權，差距也以調整後目標價計算。
#[test]
fn fill_outcome_uses_adjusted_target_for_stock_dividend() {
    let prices = closes(&[
        (15, dec!(60)),
        (16, dec!(48)),
        (17, dec!(49)),
        (18, dec!(50)),
    ]);
    let outcome =
        fill_outcome(stock_event(16, Decimal::ZERO, dec!(2)), &prices).expect("應算得出結果");
    assert_eq!(
        (outcome.pre_ex_close, outcome.fill_target),
        (dec!(60), dec!(50))
    );
    assert_eq!(outcome.trading_days_to_fill, Some(3));

    let unfilled = &prices[..3];
    let outcome =
        fill_outcome(stock_event(16, Decimal::ZERO, dec!(2)), unfilled).expect("應算得出結果");
    assert_eq!(outcome.fill_date, None);
    assert_eq!(outcome.gap_percent(), Some(dec!(2)), "(50 − 49) ÷ 50");
}

/// 現金＋配股：目標價扣配股稀釋但不扣現金；配股後當日就站上目標價算第 1 天。
#[test]
fn fill_outcome_with_cash_and_stock_dividend() {
    let prices = closes(&[(15, dec!(110)), (16, dec!(97)), (17, dec!(100))]);
    let event = stock_event(16, dec!(2), dec!(1));
    let outcome = fill_outcome(event, &prices).expect("應算得出結果");
    assert_eq!(outcome.fill_target, dec!(100));
    assert_eq!(
        outcome.trading_days_to_fill,
        Some(2),
        "97 未到 100，第 2 天 100 填息"
    );

    let same_day = closes(&[(15, dec!(110)), (16, dec!(101))]);
    let outcome = fill_outcome(event, &same_day).expect("應算得出結果");
    assert_eq!(outcome.trading_days_to_fill, Some(1));
}

/// 平均與中位數只看已填息者；偶數筆取中間兩筆平均；沒有已填息時為 None。
#[test]
fn summarize_averages_filled_events_only() {
    let outcome = |days: Option<u32>| FillOutcome {
        event: event(16),
        pre_ex_date: day(15),
        pre_ex_close: dec!(100),
        fill_target: dec!(100),
        fill_date: days.map(|_| day(20)),
        trading_days_to_fill: days,
        trading_days_since_ex: 10,
        latest_close: dec!(100),
        latest_date: day(30),
    };
    let summary = summarize(&[
        outcome(Some(1)),
        outcome(Some(10)),
        outcome(None),
        outcome(Some(2)),
        outcome(Some(4)),
    ]);
    assert_eq!(
        summary,
        FillSummary {
            events: 5,
            filled: 4,
            average_trading_days: Some(4.3),
            median_trading_days: Some(3.0),
        }
    );
    let odd = summarize(&[outcome(Some(3)), outcome(Some(1)), outcome(Some(8))]);
    assert_eq!(odd.median_trading_days, Some(3.0));
    let none = summarize(&[outcome(None)]);
    assert_eq!(
        (
            none.filled,
            none.average_trading_days,
            none.median_trading_days
        ),
        (0, None, None)
    );
}
