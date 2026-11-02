//! 訊號績效彙總：排序、四捨五入、依年度分組與全期列。

use super::*;

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
