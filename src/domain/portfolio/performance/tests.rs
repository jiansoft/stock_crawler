//! 持股績效計算的單元測試。

use rust_decimal_macros::dec;

use super::*;

fn day(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, month, day).expect("有效日期")
}

fn point(date: NaiveDate, value: Decimal, buys: Decimal, sells: Decimal) -> ValuePoint {
    ValuePoint {
        date,
        market_value: value,
        buys,
        sells,
    }
}

/// 加碼與賣出不算報酬：市值從 100 到 210，但其中 100 是新投入，實際只賺 10%。
#[test]
fn contributions_are_excluded_from_return() {
    let points = [
        point(day(1, 30), dec!(100), dec!(0), dec!(0)),
        point(day(2, 2), dec!(110), dec!(0), dec!(0)),
        point(day(2, 3), dec!(210), dec!(100), dec!(0)),
        point(day(2, 4), dec!(110), dec!(0), dec!(100)),
    ];
    let index = [
        IndexClose {
            date: day(1, 30),
            close: dec!(20000),
        },
        IndexClose {
            date: day(2, 4),
            close: dec!(21000),
        },
    ];
    let performance = evaluate(&points, &index).expect("兩天以上");
    assert_eq!(performance.return_percent, 10.0);
    assert_eq!((performance.buys, performance.sells), (100.0, 100.0));
    assert_eq!(performance.benchmark_return_percent, Some(5.0));
    assert_eq!(performance.annualized_return_percent, None);
    assert_eq!(performance.max_drawdown_percent, 0.0);
    assert_eq!(performance.max_drawdown_peak, None);

    // 1 月只有基準日、沒有報酬；2 月一個月 +10%。
    assert_eq!(performance.months.len(), 1);
    let february = &performance.months[0];
    assert_eq!(february.month, "2026-02");
    assert_eq!(
        (february.return_percent, february.net_contribution),
        (10.0, 0.0)
    );
    assert_eq!(february.end_value, 110.0);
    assert_eq!(february.benchmark_return_percent, Some(5.0));
}

/// 最大回撤以時間加權的財富指數計算，並記下高點與低點日期；月報酬逐月複利。
#[test]
fn drawdown_and_monthly_returns() {
    let points = [
        point(day(3, 31), dec!(100), dec!(0), dec!(0)),
        point(day(4, 15), dec!(120), dec!(0), dec!(0)),
        point(day(4, 30), dec!(90), dec!(0), dec!(0)),
        point(day(5, 29), dec!(108), dec!(0), dec!(0)),
    ];
    let performance = evaluate(&points, &[]).expect("兩天以上");
    assert_eq!(performance.return_percent, 8.0);
    assert_eq!(performance.max_drawdown_percent, -25.0);
    assert_eq!(performance.max_drawdown_peak, Some(day(4, 15)));
    assert_eq!(performance.max_drawdown_trough, Some(day(4, 30)));
    assert_eq!(performance.benchmark_return_percent, None);
    let returns: Vec<f64> = performance
        .months
        .iter()
        .map(|month| month.return_percent)
        .collect();
    assert_eq!(returns, vec![-10.0, 20.0]);
    assert!(performance.months[0].benchmark_return_percent.is_none());
}

/// 前一日市值為 0 時報酬記 0；滿一年才有年化報酬；少於兩天回傳 None。
#[test]
fn zero_base_and_annualized() {
    let start = NaiveDate::from_ymd_opt(2025, 1, 2).expect("有效日期");
    let points = [
        point(start, dec!(0), dec!(0), dec!(0)),
        point(day(1, 2), dec!(100), dec!(100), dec!(0)),
        point(day(1, 5), dec!(121), dec!(0), dec!(0)),
    ];
    let performance = evaluate(&points, &[]).expect("兩天以上");
    assert_eq!(performance.return_percent, 21.0);
    // 2025-01-02 到 2026-01-05 共 368 天。
    let annualized = performance.annualized_return_percent.expect("滿一年");
    assert!((20.0..21.0).contains(&annualized), "{annualized}");

    assert!(evaluate(&points[..1], &[]).is_none());
    assert!(evaluate(&[], &[]).is_none());
}

/// 持股每天剛好是大盤報酬的 2 倍再加 0.1%：beta 2、相關係數 1、alpha 為 0.1% × 245。
#[test]
fn risk_stats_regression() {
    let pairs: Vec<(f64, f64)> = (0..30)
        .map(|i| {
            let market = if i % 2 == 0 { 0.01 } else { -0.005 } * (1.0 + f64::from(i) / 30.0);
            (2.0 * market + 0.001, market)
        })
        .collect();
    let stats = risk_stats(&pairs).expect("30 天");
    assert_eq!(stats.days, 30);
    assert_eq!(stats.beta, Some(2.0));
    assert_eq!(stats.correlation, Some(1.0));
    assert_eq!(stats.alpha_percent, Some(24.5));
    // 持股波動是大盤的 2 倍，追蹤誤差等於大盤波動。
    assert!((stats.volatility_percent - 2.0 * stats.benchmark_volatility_percent).abs() < 0.02);
    assert!((stats.tracking_error_percent - stats.benchmark_volatility_percent).abs() < 0.02);

    // 不足 20 天回 None；大盤完全沒有波動時 beta、alpha、相關係數為 None。
    assert!(risk_stats(&pairs[..19]).is_none());
    let flat: Vec<(f64, f64)> = pairs.iter().map(|pair| (pair.0, 0.0)).collect();
    let stats = risk_stats(&flat).expect("30 天");
    assert_eq!(
        (stats.beta, stats.alpha_percent, stats.correlation),
        (None, None, None)
    );
    assert_eq!(stats.benchmark_volatility_percent, 0.0);
}

/// evaluate 只用持股與指數都有報酬的日子：前一日市值為 0、缺指數的日子不算。
#[test]
fn evaluate_pairs_daily_returns_with_index() {
    let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("有效日期");
    let mut points = vec![point(start, dec!(0), dec!(0), dec!(0))];
    let mut index = vec![IndexClose {
        date: start,
        close: dec!(100),
    }];
    let (mut value, mut close) = (dec!(100), dec!(100));
    for i in 1..=25_u32 {
        let date = start + chrono::Days::new(u64::from(i));
        // 第 1 天才買進（前一日市值 0），第 10 天缺指數。
        let buys = if i == 1 { dec!(100) } else { dec!(0) };
        if i > 1 {
            let up = i % 2 == 0;
            value *= if up { dec!(1.02) } else { dec!(0.99) };
            close *= if up { dec!(1.01) } else { dec!(0.995) };
        }
        points.push(point(date, value, buys, dec!(0)));
        if i != 10 {
            index.push(IndexClose { date, close });
        }
    }
    let performance = evaluate(&points, &index).expect("兩天以上");
    let risk = performance.risk.expect("足夠天數");
    // 25 天報酬：第 1 天前一日市值 0、第 10 與 11 天缺指數，剩 22 天。
    assert_eq!(risk.days, 22);
    assert_eq!(risk.beta, Some(2.0));
    assert_eq!(risk.correlation, Some(1.0));

    // 指數完全沒有資料時沒有風險指標。
    assert!(evaluate(&points, &[]).expect("兩天以上").risk.is_none());
}
