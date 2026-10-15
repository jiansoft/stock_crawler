//! 訊號績效與個股訊號歷史的期間解析、回應組裝與排序測試。

use super::*;

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

/// 外資、投信連續買超天數下限：各在 1–20，兩者都可省略。
#[test]
fn validate_streaks_bounds() {
    assert_eq!(validate_streaks(None, None), Ok(()));
    assert_eq!(validate_streaks(Some(1), Some(20)), Ok(()));
    assert_eq!(
        validate_streaks(Some(0), None),
        Err("min_foreign_streak 必須介於 1 至 20")
    );
    assert_eq!(
        validate_streaks(None, Some(21)),
        Err("min_trust_streak 必須介於 1 至 20")
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
    let benchmark = Benchmark {
        total: BenchmarkRow {
            n5: 100,
            avg5: Some(0.123_45),
            win5: Some(0.512_34),
            n240: 50,
            avg240: Some(8.765),
            win240: Some(0.6),
            ..Default::default()
        },
        years: None,
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
    assert_eq!(
        (response.min_rs_score, response.max_rs_score),
        (Some(80), None)
    );
    assert_eq!(
        (response.min_foreign_streak, response.min_trust_streak),
        (Some(3), None)
    );
    assert!(response.indicator.is_none());
    assert!(response.group_by.is_none() && response.benchmark_by_year.is_none());
    assert!(response.items.iter().all(|item| item.years.is_none()));
}

/// 依年度拆分：整段期間的列成為項目，年度列依來源與種類掛在項目下、由舊到新；
/// 大盤基準分成整段與各年度。
#[test]
fn build_performance_groups_years_under_each_item() {
    let year_row = |indicator: &str, year: i32, occurrences: i64| PerformanceRow {
        year: Some(year),
        occurrences,
        ..performance_row(indicator, "golden_cross", 3)
    };
    let rows = vec![
        year_row("kd", 2024, 4),
        performance_row("kd", "golden_cross", 3),
        year_row("kd", 2023, 6),
        year_row("macd", 2024, 1),
        performance_row("macd", "golden_cross", 3),
    ];
    let benchmark = split_benchmark(vec![
        BenchmarkRow {
            year: Some(2024),
            n5: 40,
            ..Default::default()
        },
        BenchmarkRow {
            n5: 100,
            ..Default::default()
        },
        BenchmarkRow {
            year: Some(2023),
            n5: 60,
            ..Default::default()
        },
    ]);
    let mut params = params();
    params.group_by = Some("year".to_owned());
    let response = build_performance(
        day(2023, 1, 1),
        day(2024, 12, 31),
        "all",
        &params,
        rows,
        &benchmark,
    );

    assert_eq!(response.group_by.as_deref(), Some("year"));
    assert_eq!(response.items.len(), 2);
    let kd = &response.items[0];
    assert_eq!((kd.indicator.as_str(), kd.occurrences), ("kd", 10));
    let years: Vec<(i32, i64)> = kd
        .years
        .as_ref()
        .expect("應有年度")
        .iter()
        .map(|year| (year.year, year.occurrences))
        .collect();
    assert_eq!(years, [(2023, 6), (2024, 4)]);
    assert_eq!(kd.years.as_ref().expect("應有年度")[0].horizons.len(), 5);
    assert_eq!(
        response.items[1]
            .years
            .as_ref()
            .map(|years| years.iter().map(|year| year.year).collect::<Vec<_>>()),
        Some(vec![2024])
    );

    assert_eq!(response.benchmark[0].samples, 100);
    let by_year: Vec<(i32, i64)> = response
        .benchmark_by_year
        .expect("應有各年度基準")
        .iter()
        .map(|year| (year.year, year.benchmark[0].samples))
        .collect();
    assert_eq!(by_year, [(2023, 60), (2024, 40)]);
}

/// 整段期間的列：取 `year` 為 `None` 者；沒有任何列時為全 0。
#[test]
fn total_row_picks_the_whole_period() {
    let mut yearly = performance_row("institutional_streak", "", 0);
    yearly.year = Some(2024);
    yearly.occurrences = 3;
    assert_eq!(total_row(&[yearly.clone()]).occurrences, 0);
    let whole = performance_row("institutional_streak", "", 0);
    assert_eq!(total_row(&[yearly, whole]).occurrences, 10);
    assert_eq!(total_row(&[]).occurrences, 0);
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
