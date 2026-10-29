use super::*;

fn day(offset: usize) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 1, 1).expect("固定日期") + chrono::Days::new(offset as u64)
}

/// 30 根整理（高 10、低 9、收 9.5、量 1000），最後一根依參數。
fn setup(close: f64, high: f64, volume: f64) -> (Vec<PriceBar>, Vec<VolumeBar>) {
    let mut bars: Vec<PriceBar> = (0..BASE_DAYS)
        .map(|i| PriceBar {
            date: day(i),
            close: 9.5,
            high: 10.0,
            low: 9.0,
        })
        .collect();
    bars.push(PriceBar {
        date: day(BASE_DAYS),
        close,
        high,
        low: 9.8,
    });
    let mut volumes: Vec<VolumeBar> = bars
        .iter()
        .map(|bar| VolumeBar {
            date: bar.date,
            volume: 1000.0,
        })
        .collect();
    if let Some(last) = volumes.last_mut() {
        last.volume = volume;
    }
    (bars, volumes)
}

/// 整理後帶量突破：訊號帶突破價、整理幅度、量比與距突破價。
#[test]
fn breakout_with_volume_signals() {
    let (bars, volumes) = setup(10.3, 10.5, 1600.0);

    let signal = range_breakout_signal(&bars, &volumes).expect("應出現區間突破");

    assert_eq!(signal.indicator, RANGE_BREAKOUT_INDICATOR);
    assert_eq!(signal.kind, SignalKind::Breakout);
    assert_eq!(
        signal.description,
        "收盤 10.3 帶量突破 30 日整理區間高點 10（整理幅度 11.11%，量比 1.60 倍，距突破價 3.00%）"
    );
}

/// 當天的高點不計入突破價：收盤沒超過前 30 日高點就不算突破。
#[test]
fn today_high_is_excluded_from_pivot() {
    let (bars, volumes) = setup(10.0, 10.8, 2000.0);

    assert_eq!(range_breakout_signal(&bars, &volumes), None);
}

/// 量比剛好 1.5 倍算帶量，低一點就不算。
#[test]
fn volume_ratio_boundary() {
    let (bars, volumes) = setup(10.3, 10.5, 1500.0);
    assert!(range_breakout_signal(&bars, &volumes).is_some());

    let (bars, volumes) = setup(10.3, 10.5, 1499.0);
    assert_eq!(range_breakout_signal(&bars, &volumes), None);
}

/// 距突破價剛好 5% 還算，超過就是追高。
#[test]
fn chase_boundary() {
    let (bars, volumes) = setup(10.5, 10.6, 2000.0);
    assert!(range_breakout_signal(&bars, &volumes).is_some());

    let (bars, volumes) = setup(10.51, 10.6, 2000.0);
    assert_eq!(range_breakout_signal(&bars, &volumes), None);
}

/// 整理幅度剛好 20% 還算整理，超過就不是。
#[test]
fn base_width_boundary() {
    let (mut bars, volumes) = setup(10.3, 10.5, 2000.0);
    bars[0].low = 10.0 / 1.2;
    assert!(range_breakout_signal(&bars, &volumes).is_some());

    bars[0].low = 8.3;
    assert_eq!(range_breakout_signal(&bars, &volumes), None);
}

/// 只看最近 30 根：更早的高點不影響突破價。
#[test]
fn only_last_base_days_count() {
    let (mut bars, mut volumes) = setup(10.3, 10.5, 1600.0);
    bars.insert(
        0,
        PriceBar {
            date: day(0) - chrono::Days::new(1),
            close: 20.0,
            high: 20.0,
            low: 20.0,
        },
    );
    volumes.insert(
        0,
        VolumeBar {
            date: day(0) - chrono::Days::new(1),
            volume: 1000.0,
        },
    );

    assert!(range_breakout_signal(&bars, &volumes).is_some());
}

/// 歷史不足、基準量為 0、區間底不是正數都不判斷。
#[test]
fn insufficient_or_invalid_data_is_skipped() {
    let (bars, volumes) = setup(10.3, 10.5, 1600.0);
    assert_eq!(range_breakout_signal(&bars[1..], &volumes), None);
    assert_eq!(
        range_breakout_signal(&bars, &volumes[volumes.len() - VOLUME_DAYS..]),
        None
    );

    let (bars, mut volumes) = setup(10.3, 10.5, 1600.0);
    for bar in volumes.iter_mut() {
        bar.volume = 0.0;
    }
    assert_eq!(range_breakout_signal(&bars, &volumes), None);

    let (mut bars, volumes) = setup(10.3, 10.5, 1600.0);
    // 沒有最低價時以收盤代替；收盤也是 0 時區間底不是正數。
    for bar in bars.iter_mut().take(BASE_DAYS) {
        bar.close = 0.0;
        bar.high = 0.0;
        bar.low = 0.0;
    }
    assert_eq!(range_breakout_signal(&bars, &volumes), None);
}

/// 成交量與日線的日期沒對齊（缺一天）時不判斷。
#[test]
fn misaligned_volumes_are_skipped() {
    let (bars, mut volumes) = setup(10.3, 10.5, 1600.0);
    let index = volumes.len() - 2;
    volumes[index].date = day(99);

    assert_eq!(range_breakout_signal(&bars, &volumes), None);
}
