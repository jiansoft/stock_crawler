//! ETF 折溢價與申贖警示（門檻、連續天數、過期資料與訊息格式）的測試。

use super::*;

#[test]
fn persistent_premium_requires_three_consecutive_trading_days_and_no_repeat() {
    let calendar = [day(23), day(22), day(21), day(20)];
    for rates in [[1.6, 1.7, 1.8, 0.0], [-1.6, -1.7, -1.8, 0.0]] {
        assert_eq!(
            detect_persistent(&one("0050", points(23, &rates)), day(23), &calendar).len(),
            1
        );
    }
    for rates in [
        [1.6, 1.7, 1.5, 0.0],
        [1.6, -1.7, 1.8, 0.0],
        [1.6, 1.7, 1.8, 1.9],
    ] {
        assert!(detect_persistent(&one("0050", points(23, &rates)), day(23), &calendar).is_empty());
    }
    assert!(
        detect_persistent(
            &one("0050", points(22, &[1.8, 1.8, 1.8])),
            day(23),
            &calendar
        )
        .is_empty()
    );
    assert!(
        detect_persistent(
            &one("0050", points(23, &[1.8, 1.8, 1.8])),
            day(23),
            &[day(23), day(21), day(20)]
        )
        .is_empty()
    );
}

#[test]
fn persistent_alert_is_dropped_when_symbol_already_has_premium_alert() {
    let persistent = |symbol: &str| PremiumAlert::Persistent {
        symbol: symbol.into(),
        rate: 2.1,
        average: 1.8,
    };
    let mut alerts = vec![PremiumAlert::Premium {
        symbol: "0050".into(),
        rate: 2.1,
    }];
    merge_persistent(&mut alerts, vec![persistent("0050"), persistent("0056")]);
    assert_eq!(alerts.len(), 2);
    assert_eq!(alerts[1], persistent("0056"));
}

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).expect("日期合法")
}

/// 由新到舊的折溢價，第一筆是 `latest`。
fn points(latest: u32, rates: &[f64]) -> Vec<PremiumPoint> {
    rates
        .iter()
        .enumerate()
        .map(|(i, rate)| PremiumPoint {
            date: day(latest - i as u32),
            premium_rate: *rate,
        })
        .collect()
}

fn one(symbol: &str, series: Vec<PremiumPoint>) -> HashMap<String, Vec<PremiumPoint>> {
    HashMap::from([(symbol.to_string(), series)])
}

#[test]
fn premium_alerts_only_when_crossing_the_threshold() {
    let crossed = one("00830", points(21, &[2.3, 1.1]));
    assert_eq!(
        detect(&crossed, day(21)),
        vec![PremiumAlert::Premium {
            symbol: "00830".to_string(),
            rate: 2.3
        }]
    );
    let first_ever = one("00830", points(21, &[2.0]));
    assert_eq!(detect(&first_ever, day(21)).len(), 1);
    let continuing = one("00830", points(21, &[2.3, 2.1]));
    assert!(detect(&continuing, day(21)).is_empty());
}

#[test]
fn discount_alerts_when_the_streak_just_reaches_five_days() {
    let reached = one("00679B", points(21, &[-1.2, -1.0, -1.5, -1.1, -1.2, -0.3]));
    match detect(&reached, day(21)).as_slice() {
        [PremiumAlert::Discount { rate, average, .. }] => {
            assert!((rate + 1.2).abs() < 1e-9);
            assert!((average + 1.2).abs() < 1e-9);
        }
        other => panic!("應有一則折價通知：{other:?}"),
    }
    let six_days = one("00679B", points(21, &[-1.2, -1.0, -1.5, -1.1, -1.2, -1.3]));
    assert!(detect(&six_days, day(21)).is_empty());
    let broken = one("00679B", points(21, &[-1.2, -0.9, -1.5, -1.1, -1.2]));
    assert!(detect(&broken, day(21)).is_empty());
    let short = one("00679B", points(21, &[-1.2, -1.3]));
    assert!(detect(&short, day(21)).is_empty());
}

#[test]
fn stale_data_is_ignored() {
    let stale = one("00830", points(20, &[3.0]));
    assert!(detect(&stale, day(21)).is_empty());
    assert!(detect(&one("00830", Vec::new()), day(21)).is_empty());
}

fn units(date: NaiveDate, outstanding: i64, change: i64, nav: Option<f64>) -> UnitsChangePoint {
    UnitsChangePoint {
        date,
        outstanding_units: outstanding,
        units_change: change,
        nav,
    }
}

#[test]
fn flow_alerts_when_daily_change_reaches_threshold() {
    let units = HashMap::from([
        // 前一日 1000 萬單位，淨申購 31 萬（3.1%）。
        (
            "00919".to_string(),
            units(day(21), 10_310_000, 310_000, Some(22.5)),
        ),
        // 淨贖回 4%。
        (
            "0056".to_string(),
            units(day(21), 9_600_000, -400_000, None),
        ),
        // 2.9% 不到門檻。
        (
            "0050".to_string(),
            units(day(21), 10_290_000, 290_000, Some(100.0)),
        ),
        // 不是今天。
        (
            "00878".to_string(),
            units(day(20), 2_000_000, 1_000_000, Some(20.0)),
        ),
        // 前一日單位數不為正。
        ("00999".to_string(), units(day(21), 100, 100, Some(10.0))),
    ]);
    let alerts = detect_flows(&units, day(21));
    assert_eq!(alerts.len(), 2);
    match &alerts[0] {
        PremiumAlert::Flow {
            symbol,
            ratio,
            amount,
        } => {
            assert_eq!(symbol, "0056");
            assert!((ratio + 4.0).abs() < 1e-9);
            assert!(amount.is_none());
        }
        other => panic!("應為 0056 淨贖回：{other:?}"),
    }
    let message = build_message(day(21), &alerts, |s| format!("名稱{s}")).expect("有訊息");
    assert!(
        message.contains("0056 名稱0056 單日淨贖回 \\-4\\.00%（受益單位，≥ 3%）"),
        "{message}"
    );
    assert!(
        message.contains("00919 名稱00919 單日淨申購 \\+3\\.10%（受益單位約 \\+0\\.1 億，≥ 3%）"),
        "{message}"
    );
}

#[test]
fn message_lists_alerts_in_order() {
    let alerts = vec![
        PremiumAlert::Premium {
            symbol: "00830".to_string(),
            rate: 2.31,
        },
        PremiumAlert::Discount {
            symbol: "00679B".to_string(),
            rate: -1.2,
            average: -1.25,
        },
    ];
    let message = build_message(day(21), &alerts, |s| format!("名稱{s}")).expect("有訊息");
    assert!(message.contains("持有 ETF 折溢價與申贖︰2026\\-10\\-21"));
    assert!(message.contains("00830 名稱00830 溢價 \\+2\\.31%"));
    assert!(message.contains("00679B 名稱00679B 連續 5 日折價，今日 \\-1\\.20%、平均 \\-1\\.25%"));
    assert!(build_message(day(21), &[], |_| String::new()).is_none());
}
