//! 拆出的既有回歸測試，保持原有案例與斷言。

use super::*;

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, m, d).expect("有效日期")
}

fn bar(m: u32, d: u32, close: f64) -> PriceBar {
    PriceBar {
        date: day(m, d),
        close,
        high: close,
        low: close,
    }
}

fn rule(stop_loss: Option<f64>, trailing_stop: Option<f64>) -> StopRule {
    StopRule {
        stop_loss,
        trailing_stop,
    }
}

/// 停損看相對買進價、移動停利看相對期間最高收盤；觸發回傳當天收盤的報酬，沒觸發為 None。
#[test]
fn exit_return_checks_entry_and_peak() {
    let path = [100.0, 110.0, 96.0, 89.0];
    let stop = rule(Some(10.0), None).exit_return(&path).expect("觸發");
    assert!((stop + 11.0).abs() < 1e-9, "{stop}");
    assert_eq!(rule(Some(12.0), None).exit_return(&path), None);
    // 從高點 110 跌到 96 為 −12.7%：移動停利 10% 在第二天觸發，報酬相對買進價 −4%。
    let trailing = rule(None, Some(10.0)).exit_return(&path).expect("觸發");
    assert!((trailing + 4.0).abs() < 1e-9, "{trailing}");
    // 兩者都設：先觸發的為準（移動停利在第二天，停損要到第三天）。
    let both = rule(Some(10.0), Some(10.0))
        .exit_return(&path)
        .expect("觸發");
    assert!((both + 4.0).abs() < 1e-9, "{both}");
    assert_eq!(rule(Some(10.0), None).exit_return(&[]), None);
    assert_eq!(rule(Some(10.0), None).exit_return(&[0.0, -1.0]), None);
    assert!(!StopRule::default().is_active());
    assert_eq!(StopRule::default().exit_return(&path), None);
}

/// 可觸發的收盤：換股日（沒成交時取之前最近一根）之後到下一期換股日前一天；沒有下一期時取
/// `hold_days` − 1 根；換股日之前沒有日線為空。
#[test]
fn holding_path_spans_until_next_rebalance() {
    let bars = [
        bar(1, 2, 10.0),
        bar(1, 4, 11.0),
        bar(1, 5, 12.0),
        bar(1, 8, 13.0),
        bar(1, 9, 14.0),
    ];
    assert_eq!(
        holding_path(&bars, day(1, 3), Some(day(1, 8)), 5),
        [10.0, 11.0, 12.0]
    );
    assert_eq!(holding_path(&bars, day(1, 4), None, 3), [11.0, 12.0, 13.0]);
    assert!(holding_path(&bars, day(1, 1), None, 3).is_empty());
    assert_eq!(holding_path(&bars, day(1, 9), None, 3), [14.0]);
}

/// 觸發的持股報酬改為出場報酬並重算平均；未滿持有天數的期數、沒有報酬或沒有日線的持股不動；
/// 沒有設規則時完全不變。
#[test]
fn apply_stops_rewrites_triggered_returns() {
    let period = |d: u32, symbols: &[&str], returns: &[Option<f64>]| PeriodRow {
        date: day(1, d),
        holdings: symbols.len() as i64,
        symbols: symbols.iter().map(|symbol| (*symbol).to_owned()).collect(),
        returns: returns.to_vec(),
        avg_return: Some(0.0),
        ..PeriodRow::default()
    };
    let mut rows = vec![
        period(2, &["A", "B", "C"], &[Some(-15.0), Some(5.0), None]),
        PeriodRow {
            pending: true,
            ..period(9, &["A"], &[None])
        },
    ];
    let series = |closes: &[(u32, f64)]| {
        Arc::new(
            closes
                .iter()
                .map(|(d, close)| bar(1, *d, *close))
                .collect::<Vec<_>>(),
        )
    };
    let bars: HashMap<String, Arc<Vec<PriceBar>>> = [
        (
            "A".to_owned(),
            series(&[(2, 100.0), (3, 92.0), (4, 88.0), (9, 85.0), (10, 50.0)]),
        ),
        ("B".to_owned(), series(&[(2, 50.0), (3, 49.0), (9, 52.5)])),
        ("C".to_owned(), series(&[(2, 10.0), (3, 1.0)])),
    ]
    .into_iter()
    .collect();

    let original = rows.clone();
    apply_stops(&mut rows, &bars, StopRule::default(), 5);
    assert_eq!(rows, original);

    apply_stops(&mut rows, &bars, rule(Some(10.0), None), 5);
    let stopped = rows[0].returns[0].expect("停損報酬");
    assert!((stopped + 12.0).abs() < 1e-9, "{stopped}");
    assert_eq!(rows[0].returns[1..], [Some(5.0), None]);
    assert_eq!(rows[0].stopped, [true, false, false]);
    let avg = rows[0].avg_return.expect("平均報酬");
    assert!((avg + 3.5).abs() < 1e-9, "{avg}");
    assert_eq!(rows[1], original[1]);
}
