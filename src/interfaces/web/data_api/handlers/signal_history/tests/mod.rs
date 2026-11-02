//! 訊號績效與個股訊號歷史的期間解析、回應組裝與排序測試。

use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::benchmark::{Benchmark, split_benchmark};
use super::performance::build_performance;
use super::rows::{BenchmarkRow, StockSignalRow};
use super::stock_signals::build_stock_signals;
use super::validation::require_indicator_for_long_period;
use super::*;
use crate::interfaces::web::data_api::dto::{BenchmarkStat, HorizonStat, SignalPerformanceParams};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("有效日期")
}

fn performance_row(indicator: &str, kind: &str, n60: i64) -> PerformanceRow {
    PerformanceRow {
        indicator: indicator.to_owned(),
        kind: kind.to_owned(),
        year: None,
        occurrences: 10,
        n5: 10,
        avg5: Some(1.234_56),
        med5: Some(0.996),
        win5: Some(0.666_666),
        excess5: Some(-0.333_3),
        beat5: Some(0.5),
        n20: 8,
        avg20: Some(2.0),
        med20: None,
        win20: Some(0.75),
        excess20: None,
        beat20: None,
        n60,
        avg60: (n60 > 0).then_some(5.0),
        med60: None,
        win60: None,
        excess60: None,
        beat60: None,
        n120: 0,
        avg120: None,
        med120: None,
        win120: None,
        excess120: None,
        beat120: None,
        n240: 2,
        avg240: Some(30.0),
        med240: Some(30.0),
        win240: Some(1.0),
        excess240: Some(12.345),
        beat240: Some(0.5),
    }
}

fn params() -> SignalPerformanceParams {
    SignalPerformanceParams {
        from: None,
        to: None,
        market: None,
        kind: Some("golden_cross".to_owned()),
        indicator: None,
        industry: Some("半導體業".to_owned()),
        min_rs_score: Some(80),
        max_rs_score: None,
        min_foreign_streak: Some(3),
        min_trust_streak: None,
        group_by: None,
    }
}

/// `group_by`：省略不拆分、`year` 依年度，其他值回錯誤。
#[test]
fn parse_group_by_accepts_only_year() {
    assert_eq!(parse_group_by(None), Ok(false));
    assert_eq!(parse_group_by(Some("year")), Ok(true));
    assert_eq!(parse_group_by(Some("month")), Err("group_by 只支援 year"));
}

/// 外資、投信連續買超天數下限：各在 1–60，兩者都可省略。
#[test]
fn validate_streaks_bounds() {
    assert_eq!(validate_streaks(None, None), Ok(()));
    assert_eq!(validate_streaks(Some(1), Some(60)), Ok(()));
    assert_eq!(
        validate_streaks(Some(0), None),
        Err("min_foreign_streak 必須介於 1 至 60")
    );
    assert_eq!(
        validate_streaks(None, Some(61)),
        Err("min_trust_streak 必須介於 1 至 60")
    );
}

/// 相對強度上下限：各在 1–99、下限不大於上限，兩者都可省略。
#[test]
fn validate_rs_range_bounds() {
    assert_eq!(validate_rs_range(None, None), Ok(()));
    assert_eq!(validate_rs_range(Some(1), Some(99)), Ok(()));
    assert_eq!(validate_rs_range(Some(80), Some(80)), Ok(()));
    assert_eq!(
        validate_rs_range(Some(0), None),
        Err("min_rs_score 必須介於 1 至 99")
    );
    assert_eq!(
        validate_rs_range(None, Some(100)),
        Err("max_rs_score 必須介於 1 至 99")
    );
    assert_eq!(
        validate_rs_range(Some(81), Some(80)),
        Err("min_rs_score 不可大於 max_rs_score")
    );
}

/// 起日預設迄日往前 364 天；起日晚於迄日或期間超過 3,653 天回錯誤。
#[test]
fn resolve_period_defaults_and_limits() {
    let to = day(2026, 10, 13);
    assert_eq!(resolve_period(None, to), Ok(day(2025, 10, 14)));
    assert_eq!(resolve_period(Some(to), to), Ok(to));
    assert_eq!(
        resolve_period(Some(day(2016, 10, 13)), to),
        Ok(day(2016, 10, 13))
    );
    assert_eq!(
        resolve_period(Some(day(2016, 10, 12)), to),
        Err("統計期間最長 3,653 天")
    );
    assert_eq!(
        resolve_period(Some(day(2026, 10, 14)), to),
        Err("from 不可晚於 to")
    );
}

/// 超過 1,096 天時必須指定訊號來源；1,096 天以內不需要。
#[test]
fn long_periods_require_an_indicator() {
    let to = day(2026, 10, 13);
    assert_eq!(
        require_indicator_for_long_period(day(2023, 10, 14), to, None),
        Ok(())
    );
    assert_eq!(
        require_indicator_for_long_period(day(2023, 10, 13), to, None),
        Err("統計期間超過 1,096 天時必須指定 indicator")
    );
    assert_eq!(
        require_indicator_for_long_period(day(2019, 1, 2), to, Some("52_week")),
        Ok(())
    );
}

fn stock_row(date: NaiveDate, indicator: &str, kind: &str) -> StockSignalRow {
    StockSignalRow {
        date,
        indicator: indicator.to_owned(),
        kind: kind.to_owned(),
        description: format!("{indicator} {kind}"),
        closing_price: Some(Decimal::new(1005, 1)),
        change_range: Some(Decimal::new(-123, 2)),
        return_5d: Some(Decimal::new(25, 1)),
        return_20d: None,
        return_60d: None,
        return_120d: None,
        return_240d: Some(Decimal::new(-405, 1)),
        total: 7,
    }
}

/// 由新到舊；同一天依來源、種類的固定順序；total 取自查詢的視窗計數，空清單為 0。
#[test]
fn build_stock_signals_orders_items() {
    let rows = vec![
        stock_row(day(2026, 10, 12), "kd", "golden_cross"),
        stock_row(day(2026, 10, 13), "52_week", "new_high"),
        stock_row(day(2026, 10, 13), "ma5_ma20", "golden_cross"),
    ];
    let response =
        build_stock_signals("2330".to_owned(), day(2026, 4, 17), day(2026, 10, 13), rows);
    let order: Vec<(&str, &str)> = response
        .items
        .iter()
        .map(|item| (item.date.as_str(), item.indicator.as_str()))
        .collect();
    assert_eq!(
        order,
        [
            ("2026-10-13", "ma5_ma20"),
            ("2026-10-13", "52_week"),
            ("2026-10-12", "kd"),
        ]
    );
    assert_eq!(response.total, 7);
    assert!(response.adjusted);
    let first = &response.items[0];
    assert_eq!(first.closing_price, Some(100.5));
    assert_eq!(first.change_percent, Some(-1.23));
    assert_eq!(first.return_5d, Some(2.5));
    assert_eq!(first.return_20d, None);
    assert_eq!(first.return_120d, None);
    assert_eq!(first.return_240d, Some(-40.5));

    let empty = build_stock_signals("2330".to_owned(), day(2026, 1, 1), day(2026, 1, 2), vec![]);
    assert_eq!(empty.total, 0);
    assert!(empty.items.is_empty());
}

mod performance;
