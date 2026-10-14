//! 訊號績效與個股訊號歷史的期間解析、回應組裝與排序測試。

use super::*;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("有效日期")
}

fn performance_row(indicator: &str, kind: &str, n60: i64) -> PerformanceRow {
    PerformanceRow {
        indicator: indicator.to_owned(),
        kind: kind.to_owned(),
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
    }
}

/// 起日預設迄日往前 364 天；起日晚於迄日或期間超過 1,096 天回錯誤。
#[test]
fn resolve_period_defaults_and_limits() {
    let to = day(2026, 10, 13);
    assert_eq!(resolve_period(None, to), Ok(day(2025, 10, 14)));
    assert_eq!(resolve_period(Some(to), to), Ok(to));
    assert_eq!(
        resolve_period(Some(day(2023, 10, 14)), to),
        Ok(day(2023, 10, 14))
    );
    assert_eq!(
        resolve_period(Some(day(2023, 10, 13)), to),
        Err("統計期間最長 1,096 天")
    );
    assert_eq!(
        resolve_period(Some(day(2026, 10, 14)), to),
        Err("from 不可晚於 to")
    );
}

/// 項目依來源、種類的固定順序；數值四捨五入；天期與基準依 5、20、60 排列。
#[test]
fn build_performance_orders_and_rounds() {
    let rows = vec![
        performance_row("52_week", "new_high", 0),
        performance_row("kd", "oversold", 3),
        performance_row("kd", "golden_cross", 3),
        performance_row("ma5_ma20", "death_cross", 3),
    ];
    let benchmark = BenchmarkRow {
        n5: 100,
        avg5: Some(0.123_45),
        win5: Some(0.512_34),
        n240: 50,
        avg240: Some(8.765),
        win240: Some(0.6),
        ..Default::default()
    };
    let response = build_performance(
        day(2025, 10, 14),
        day(2026, 10, 13),
        "all",
        &params(),
        rows,
        &benchmark,
    );

    let order: Vec<(&str, &str)> = response
        .items
        .iter()
        .map(|item| (item.indicator.as_str(), item.kind.as_str()))
        .collect();
    assert_eq!(
        order,
        [
            ("ma5_ma20", "death_cross"),
            ("kd", "golden_cross"),
            ("kd", "oversold"),
            ("52_week", "new_high"),
        ]
    );
    let first = &response.items[0].horizons;
    assert_eq!(
        first[0],
        HorizonStat {
            days: 5,
            samples: 10,
            avg_return: Some(1.23),
            median_return: Some(1.0),
            win_rate: Some(0.6667),
            avg_excess_return: Some(-0.33),
            beat_market_rate: Some(0.5),
        }
    );
    assert_eq!(first[1].days, 20);
    assert_eq!(first[1].median_return, None);
    let last = &response.items[3].horizons[2];
    assert_eq!((last.days, last.samples, last.avg_return), (60, 0, None));
    let days: Vec<u32> = first.iter().map(|stat| stat.days).collect();
    assert_eq!(days, [5, 20, 60, 120, 240]);
    assert_eq!(
        first[4],
        HorizonStat {
            days: 240,
            samples: 2,
            avg_return: Some(30.0),
            median_return: Some(30.0),
            win_rate: Some(1.0),
            avg_excess_return: Some(12.35),
            beat_market_rate: Some(0.5),
        }
    );
    assert_eq!((first[3].samples, first[3].avg_return), (0, None));

    let empty = |days| BenchmarkStat {
        days,
        samples: 0,
        avg_return: None,
        win_rate: None,
    };
    assert_eq!(
        response.benchmark,
        [
            BenchmarkStat {
                days: 5,
                samples: 100,
                avg_return: Some(0.12),
                win_rate: Some(0.5123),
            },
            empty(20),
            empty(60),
            empty(120),
            BenchmarkStat {
                days: 240,
                samples: 50,
                avg_return: Some(8.77),
                win_rate: Some(0.6),
            },
        ]
    );
    assert_eq!(response.from, "2025-10-14");
    assert_eq!(response.to, "2026-10-13");
    assert!(response.adjusted);
    assert_eq!(response.kind.as_deref(), Some("golden_cross"));
    assert_eq!(response.industry.as_deref(), Some("半導體業"));
    assert!(response.indicator.is_none());
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
