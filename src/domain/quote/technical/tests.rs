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

/// 均線、KD、MACD 的交叉與收盤價站上／跌破均線。
#[test]
fn signals_detect_crosses() {
    let mut prev = point(100.0);
    let mut last = point(110.0);
    // MA5 由 ≤ MA20 變 >：黃金交叉；MA20 由 ≥ MA60 變 <：死亡交叉
    prev.moving_averages = [
        Some(99.0),
        None,
        Some(100.0),
        Some(100.0),
        None,
        Some(105.0),
    ];
    last.moving_averages = [
        Some(102.0),
        None,
        Some(101.0),
        Some(102.0),
        None,
        Some(108.0),
    ];
    prev.k = Some(15.0);
    prev.d = Some(18.0);
    last.k = Some(22.0);
    last.d = Some(20.0);
    prev.macd_dif = Some(1.0);
    prev.macd_signal = Some(0.5);
    last.macd_dif = Some(0.2);
    last.macd_signal = Some(0.4);

    let found = signals(Some(&prev), &last);
    let kinds: Vec<(&str, SignalKind)> = found.iter().map(|s| (s.indicator, s.kind)).collect();
    assert_eq!(
        kinds,
        [
            ("ma5_ma20", SignalKind::GoldenCross),
            ("ma20_ma60", SignalKind::DeathCross),
            ("kd", SignalKind::GoldenCross),
            ("macd", SignalKind::DeathCross),
            ("close_ma20", SignalKind::CrossAbove),
            ("close_ma60", SignalKind::CrossAbove),
            ("close_ma240", SignalKind::CrossAbove),
        ]
    );
    assert_eq!(
        found[0].description,
        "5 日均線（週線）向上穿越 20 日均線（月線）（黃金交叉）"
    );
    assert_eq!(
        found[2].description,
        "K 值向上穿越 D 值，K＝22.00（黃金交叉）"
    );
    assert_eq!(found[4].description, "收盤價站上 20 日均線（月線）");

    // 反向：收盤跌破
    let below = signals(Some(&last), &{
        let mut p = last.clone();
        p.close = 90.0;
        p
    });
    assert!(
        below
            .iter()
            .any(|s| s.indicator == "close_ma20" && s.kind == SignalKind::CrossBelow)
    );
    assert_eq!(ma_name(120), "120 日均線");
}

/// 沒有前一日只看超買超賣；RSI 與 K 值的門檻含等號。
#[test]
fn signals_report_overbought_and_oversold() {
    let mut last = point(10.0);
    last.rsi14 = Some(70.0);
    last.k = Some(20.0);
    let found = signals(None, &last);
    assert_eq!(found.len(), 2);
    assert_eq!(
        (found[0].indicator, found[0].kind),
        ("rsi14", SignalKind::Overbought)
    );
    assert_eq!(found[0].description, "RSI(14) 70.00，位於超買區（門檻 70）");
    assert_eq!(
        (found[1].indicator, found[1].kind),
        ("kd", SignalKind::Oversold)
    );

    last.rsi14 = Some(50.0);
    last.k = Some(50.0);
    assert!(signals(None, &last).is_empty());
    assert!(signals(Some(&point(10.0)), &point(10.0)).is_empty());
}

/// 對外名稱固定，且可由名稱解析回原值。
#[test]
fn signal_kind_names_are_stable() {
    let names = SIGNAL_KINDS.map(SignalKind::as_str);
    assert_eq!(
        names,
        [
            "golden_cross",
            "death_cross",
            "cross_above",
            "cross_below",
            "overbought",
            "oversold",
            "new_high",
            "new_low",
            "breakout"
        ]
    );
    for kind in SIGNAL_KINDS {
        assert_eq!(SignalKind::parse(kind.as_str()), Some(kind));
    }
    assert_eq!(SignalKind::parse("unknown"), None);
}

/// 資料表的名稱可還原成訊號；未知名稱回 None。
#[test]
fn technical_signal_from_names() {
    let signal =
        TechnicalSignal::from_names("52_week", "new_high", "說明".to_owned()).expect("已知名稱");
    assert_eq!(signal.indicator, FIFTY_TWO_WEEK_INDICATOR);
    assert_eq!(signal.kind, SignalKind::NewHigh);
    assert_eq!(signal.description, "說明");
    assert!(TechnicalSignal::from_names("ma1", "new_high", String::new()).is_none());
    assert!(TechnicalSignal::from_names("kd", "breakdown", String::new()).is_none());
}

/// 前 52 週的日線：每 7 天 5 個交易日、共 `days` 根，最高 10、最低 5，最後加上當日。
fn year_bars(days: usize, last_high: f64, last_low: f64) -> Vec<PriceBar> {
    let start = NaiveDate::from_ymd_opt(2025, 1, 6).expect("有效日期");
    let mut out: Vec<PriceBar> = (0..days)
        .map(|i| PriceBar {
            date: start + chrono::Days::new((i / 5 * 7 + i % 5) as u64),
            close: 8.0,
            high: 10.0,
            low: 5.0,
        })
        .collect();
    let last_date = out.last().expect("至少一根").date + chrono::Days::new(1);
    out.push(PriceBar {
        date: last_date,
        close: 8.0,
        high: last_high,
        low: last_low,
    });
    out
}

/// 當日最高價高於前 52 週最高價為新高、最低價低於前 52 週最低價為新低；等於不算。
#[test]
fn fifty_two_week_signals_compare_with_prior_year() {
    let both = fifty_two_week_signals(&year_bars(250, 10.5, 4.5));
    let kinds: Vec<SignalKind> = both.iter().map(|signal| signal.kind).collect();
    assert_eq!(kinds, [SignalKind::NewHigh, SignalKind::NewLow]);
    assert!(both.iter().all(|signal| signal.indicator == "52_week"));
    assert_eq!(both[0].description, "最高價 10.5 創 52 週新高（前高 10）");
    assert_eq!(both[1].description, "最低價 4.5 創 52 週新低（前低 5）");

    assert!(fifty_two_week_signals(&year_bars(250, 10.0, 5.0)).is_empty());
    assert!(fifty_two_week_signals(&[]).is_empty());
}

/// 前 52 週不足 200 個交易日不判斷；超過 364 天的舊資料不列入比較。
#[test]
fn fifty_two_week_signals_need_enough_recent_history() {
    assert!(fifty_two_week_signals(&year_bars(199, 99.0, 1.0)).is_empty());
    assert_eq!(fifty_two_week_signals(&year_bars(200, 99.0, 1.0)).len(), 2);

    // 一年多以前的極端高低價不影響：只看前 364 天。
    let mut bars = year_bars(300, 10.5, 4.5);
    bars[0].high = 50.0;
    bars[0].low = 1.0;
    assert_eq!(fifty_two_week_signals(&bars).len(), 2);
}

/// 最低價為 0（缺值）的日子不參與新低比較；當日最低價為 0 也不算新低。
#[test]
fn fifty_two_week_low_ignores_missing_prices() {
    let mut bars = year_bars(250, 10.0, 4.5);
    let len = bars.len();
    bars[len - 2].low = 0.0;
    let signals = fifty_two_week_signals(&bars);
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].kind, SignalKind::NewLow);

    let zero_low = year_bars(250, 10.0, 0.0);
    assert!(fifty_two_week_signals(&zero_low).is_empty());
}

fn adjustment(date: NaiveDate, cash: f64, stock: f64, share_ratio: f64) -> PriceAdjustment {
    PriceAdjustment {
        date,
        cash_dividend: cash,
        stock_dividend: stock,
        share_ratio,
    }
}

/// 還原因子＝參考價 ÷ 前收：除息、除權、減資與分割；不合理或不需調整時為 None。
#[test]
fn price_adjustment_factor_follows_reference_price() {
    let factor = |cash, stock, ratio| adjustment(day(0), cash, stock, ratio).factor(100.0);
    assert_eq!(factor(5.0, 0.0, 1.0), Some(0.95));
    // 配股 1 元（每股配 0.1 股）：參考價 100 ÷ 1.1。
    assert!((factor(0.0, 1.0, 1.0).expect("有因子") - 1.0 / 1.1).abs() < 1e-12);
    // 減資五成（1 股變 0.5 股）價格加倍；1 拆 4 價格變四分之一。
    assert_eq!(factor(0.0, 0.0, 0.5), Some(2.0));
    assert_eq!(factor(0.0, 0.0, 4.0), Some(0.25));
    assert_eq!(factor(0.0, 0.0, 1.0), None);
    assert_eq!(factor(100.0, 0.0, 1.0), None);
    assert_eq!(factor(0.0, 0.0, 0.0), None);
    assert_eq!(adjustment(day(0), 1.0, 0.0, 1.0).factor(0.0), None);
}

/// 事件日之前的價格乘上因子、最新一根不變；多個事件累乘；
/// 截止日之後與第一根（含）之前的事件不採用；事件日沒成交時套用到前一根（含）之前。
#[test]
fn adjust_bars_scales_prices_before_events() {
    let raw = bars(&[10.0, 10.0, 9.0, 9.0, 18.0]);
    let events = [
        adjustment(day(2), 1.0, 0.0, 1.0),
        adjustment(day(4), 0.0, 0.0, 0.5),
        adjustment(day(9), 3.0, 0.0, 1.0),
        adjustment(day(0), 3.0, 0.0, 1.0),
    ];
    // 除息 1 元：前收 10 → 參考價 9（0.9）；減資五成：前收 9 → 18（2 倍）。
    assert_eq!(adjustment_factors(&raw, &events), [1.8, 1.8, 2.0, 2.0, 1.0]);
    assert!(adjustment_factors(&[], &events).is_empty());
    let adjusted = adjust_bars(&raw, &events);
    let closes: Vec<f64> = adjusted.iter().map(|bar| bar.close).collect();
    assert_eq!(closes, [18.0, 18.0, 18.0, 18.0, 18.0]);
    assert_eq!(adjusted[0].high, 18.0);
    assert_eq!(adjusted[0].low, 18.0);
    assert!(adjusted.iter().zip(&raw).all(|(a, r)| a.date == r.date));

    // 事件日（day 2）沒有成交：以 day 1 收盤計算，day 1（含）之前調整。
    let gap = vec![raw[0], raw[1], raw[3]];
    let adjusted = adjust_bars(&gap, &[adjustment(day(2), 1.0, 0.0, 1.0)]);
    let closes: Vec<f64> = adjusted.iter().map(|bar| bar.close).collect();
    assert_eq!(closes, [9.0, 9.0, 9.0]);

    assert!(adjust_bars(&[], &[adjustment(day(2), 1.0, 0.0, 1.0)]).is_empty());
    assert_eq!(adjust_bars(&raw, &[]), raw);
}

/// 還原後除息跳空不再被誤判成 52 週新低；價格描述四捨五入到小數兩位。
#[test]
fn adjusted_bars_avoid_false_fifty_two_week_low() {
    let mut raw = year_bars(250, 10.0, 7.0);
    let ex_date = raw.last().expect("至少一根").date;
    // 除息 3 元：前收 8 → 參考價 5。當日最低 4.9 低於未還原的前低 5，
    // 還原後前低為 5 × 5/8 = 3.125，不算新低。
    raw.last_mut().expect("至少一根").low = 4.9;
    let unadjusted = fifty_two_week_signals(&raw);
    assert!(unadjusted.iter().any(|s| s.kind == SignalKind::NewLow));

    let adjusted = adjust_bars(&raw, &[adjustment(ex_date, 3.0, 0.0, 1.0)]);
    assert!(
        !fifty_two_week_signals(&adjusted)
            .iter()
            .any(|s| s.kind == SignalKind::NewLow)
    );

    assert_eq!(price_text(3.126), "3.13");
    assert_eq!(price_text(256.5), "256.5");
    assert_eq!(price_text(100.0), "100");
    assert_eq!(price_text(1.0 / 3.0), "0.33");
}
