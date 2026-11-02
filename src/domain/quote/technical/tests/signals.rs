//! 技術訊號偵測：交叉、超買超賣與名稱轉換。

use super::*;

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
            "breakout",
            "failed"
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
