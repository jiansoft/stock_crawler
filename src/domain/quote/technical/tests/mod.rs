//! 技術指標計算、還原權值與訊號判斷的純函式測試。

use chrono::NaiveDate;

use super::detection::{ma_name, price_text};
use super::indicators::{bar_high, bar_low, macd, rsi};
use super::*;

fn day(offset: usize) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 1, 1).expect("有效日期") + chrono::Days::new(offset as u64)
}

/// 只有收盤價的日線（最高、最低等於收盤）。
fn bars(closes: &[f64]) -> Vec<PriceBar> {
    closes
        .iter()
        .enumerate()
        .map(|(i, &close)| PriceBar {
            date: day(i),
            close,
            high: close,
            low: close,
        })
        .collect()
}

fn approx(actual: Option<f64>, expected: f64) {
    let actual = actual.expect("應有值");
    assert!(
        (actual - expected).abs() < 1e-9,
        "actual {actual} expected {expected}"
    );
}

fn point(close: f64) -> IndicatorPoint {
    IndicatorPoint {
        date: day(0),
        close,
        moving_averages: [None; MA_PERIODS.len()],
        rsi6: None,
        rsi12: None,
        rsi14: None,
        k: None,
        d: None,
        macd_dif: None,
        macd_signal: None,
        macd_histogram: None,
    }
}

/// 均線在滿 N 日後才有值，乖離率以收盤相對均線計算。
#[test]
fn moving_averages_need_full_window() {
    let closes: Vec<f64> = (1..=20).map(f64::from).collect();
    let points = compute(&bars(&closes));
    assert_eq!(points.len(), 20);
    assert!(points[3].moving_average(5).is_none());
    approx(points[4].moving_average(5), 3.0);
    approx(points[19].moving_average(5), 18.0);
    approx(points[19].moving_average(10), 15.5);
    approx(points[19].moving_average(20), 10.5);
    assert!(points[19].moving_average(60).is_none());
    assert!(points[19].moving_average(7).is_none());
    approx(points[19].bias(5), (20.0 - 18.0) / 18.0 * 100.0);
    assert!(points[19].bias(60).is_none());
    assert!(compute(&[]).is_empty());
}

/// RSI 首值為簡單平均，之後 Wilder 平滑；只漲為 100、全平為 50。
#[test]
fn rsi_uses_wilder_smoothing() {
    // 漲跌：+2, -1, +2, -1, +2, -1, -3
    let closes = [10.0, 12.0, 11.0, 13.0, 12.0, 14.0, 13.0, 10.0];
    let values = rsi(&closes, 6);
    assert!(values[5].is_none());
    // 前 6 日平均漲 1、平均跌 0.5 → RS 2 → 66.67
    approx(values[6], 100.0 - 100.0 / 3.0);
    // 第 7 日跌 3：平均漲 5/6、平均跌 (0.5×5+3)/6
    let (gain, loss) = (5.0 / 6.0, (0.5 * 5.0 + 3.0) / 6.0);
    approx(values[7], 100.0 - 100.0 / (1.0 + gain / loss));

    let rising: Vec<f64> = (1..=8).map(f64::from).collect();
    approx(rsi(&rising, 6)[7], 100.0);
    approx(rsi(&[5.0; 8], 6)[7], 50.0);
}

/// KD 自第 9 日起有值；高低相同取 RSV 50，缺高低價時以收盤代替。
#[test]
fn kd_follows_rsv_smoothing() {
    let flat = compute(&bars(&[10.0; 9]));
    assert!(flat[7].k.is_none());
    approx(flat[8].k, 50.0);
    approx(flat[8].d, 50.0);

    // 九日區間 1～9，收在最高 → RSV 100 → K 66.67、D 55.56
    let mut rising = bars(&(1..=9).map(f64::from).collect::<Vec<_>>());
    rising[3].high = 0.0;
    rising[4].low = 0.0;
    let points = compute(&rising);
    approx(points[8].k, 50.0 * 2.0 / 3.0 + 100.0 / 3.0);
    approx(points[8].d, 50.0 * 2.0 / 3.0 + (200.0 / 3.0) / 3.0);
}

/// 缺值的最高、最低價以收盤價代替。
#[test]
fn bar_extremes_fall_back_to_close() {
    let bar = PriceBar {
        date: day(0),
        close: 10.0,
        high: 0.0,
        low: 0.0,
    };
    assert_eq!((bar_high(&bar), bar_low(&bar)), (10.0, 10.0));
    let wide = PriceBar {
        high: 12.0,
        low: 9.0,
        ..bar
    };
    assert_eq!((bar_high(&wide), bar_low(&wide)), (12.0, 9.0));
}

/// MACD：DIF 自第 26 日、訊號線自第 34 日起有值；平盤時都是 0。
#[test]
fn macd_warms_up_before_reporting() {
    let flat = compute(&bars(&[10.0; 40]));
    assert!(flat[24].macd_dif.is_none());
    approx(flat[25].macd_dif, 0.0);
    assert!(flat[32].macd_signal.is_none());
    approx(flat[33].macd_signal, 0.0);
    approx(flat[39].macd_histogram, 0.0);
    assert!(flat[32].macd_histogram.is_none());

    // 持續上漲時快線在慢線之上
    let rising: Vec<f64> = (1..=40).map(f64::from).collect();
    let points = compute(&bars(&rising));
    assert!(points[39].macd_dif.expect("有 DIF") > 0.0);

    // 不足 26 日沒有 DIF；空輸入回空結果
    let (dif, _) = macd(&[10.0, 12.0]);
    assert!(dif.iter().all(Option::is_none));
    assert!(macd(&[]).0.is_empty());
}

mod signals;

mod fifty_two_week;
