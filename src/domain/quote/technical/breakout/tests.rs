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

/// 沒有突破時也回傳整理區間的中間值，供個股技術頁顯示。
#[test]
fn state_reports_values_without_breakout() {
    let (bars, volumes) = setup(9.8, 9.9, 800.0);

    let state = range_breakout_state(&bars, &volumes).expect("資料足夠");

    assert!(!state.breaks_out);
    assert_eq!((state.pivot, state.floor), (10.0, 9.0));
    assert!((state.volume_ratio - 0.8).abs() < 1e-9, "{state:?}");
    assert!((state.distance_percent + 2.0).abs() < 1e-9, "{state:?}");
    assert!(
        (state.base_width_percent - 100.0 / 9.0).abs() < 1e-9,
        "{state:?}"
    );
}

/// 第 30 根帶量突破（收 10.3、突破價 10），之後依序接上 `after` 的收盤（高低等於收盤、量 1000）。
fn breakout_then(after: &[f64]) -> (Vec<PriceBar>, Vec<VolumeBar>) {
    let (mut bars, mut volumes) = setup(10.3, 10.5, 1600.0);
    for (i, &close) in after.iter().enumerate() {
        let date = day(BASE_DAYS + 1 + i);
        bars.push(PriceBar {
            date,
            close,
            high: close,
            low: close,
        });
        volumes.push(VolumeBar {
            date,
            volume: 1000.0,
        });
    }
    (bars, volumes)
}

/// 突破後守住兩天、第三天收在突破價之下：記假突破，說明附突破日、突破價與第幾個交易日。
#[test]
fn falling_back_below_pivot_is_failed_breakout() {
    let (bars, volumes) = breakout_then(&[10.2, 10.0, 9.9]);

    let signal = failed_breakout_signal(&bars, &volumes).expect("應出現假突破");

    assert_eq!(signal.indicator, RANGE_BREAKOUT_INDICATOR);
    assert_eq!(signal.kind, SignalKind::Failed);
    assert_eq!(
        signal.description,
        "收盤 9.9 跌回 2026-01-31 區間突破的突破價 10 之下（突破後第 3 個交易日）"
    );
    let all: Vec<SignalKind> = range_breakout_signals(&bars, &volumes)
        .map(|signal| signal.kind)
        .collect();
    assert_eq!(all, [SignalKind::Failed]);
}

/// 突破隔天就跌回也算；收在突破價上（含等於）不算。
#[test]
fn next_day_fall_back_counts_and_pivot_close_holds() {
    let (bars, volumes) = breakout_then(&[9.95]);
    assert!(failed_breakout_signal(&bars, &volumes).is_some());

    let (bars, volumes) = breakout_then(&[10.0]);
    assert_eq!(failed_breakout_signal(&bars, &volumes), None);
}

/// 同一次突破只記第一次跌回：之前已跌回過（即使又站回去）就不再記。
#[test]
fn only_first_fall_back_is_recorded() {
    let (bars, volumes) = breakout_then(&[9.9, 9.8]);
    assert_eq!(failed_breakout_signal(&bars, &volumes), None);

    let (bars, volumes) = breakout_then(&[9.9, 10.2, 9.8]);
    assert_eq!(failed_breakout_signal(&bars, &volumes), None);
}

/// 突破後第 10 個交易日跌回還算，第 11 個交易日才跌回不算。
#[test]
fn fall_back_window_is_ten_trading_days() {
    let mut after = vec![10.2; FAILED_WITHIN_DAYS - 1];
    after.push(9.9);
    let (bars, volumes) = breakout_then(&after);
    assert!(failed_breakout_signal(&bars, &volumes).is_some());

    let mut after = vec![10.2; FAILED_WITHIN_DAYS];
    after.push(9.9);
    let (bars, volumes) = breakout_then(&after);
    assert_eq!(failed_breakout_signal(&bars, &volumes), None);
}

/// 突破當天本身不是假突破；成交量沒對齊時看不出突破，也就沒有假突破。
#[test]
fn breakout_day_and_misaligned_volumes_are_not_failed() {
    let (bars, volumes) = breakout_then(&[]);
    assert_eq!(failed_breakout_signal(&bars, &volumes), None);
    let all: Vec<SignalKind> = range_breakout_signals(&bars, &volumes)
        .map(|signal| signal.kind)
        .collect();
    assert_eq!(all, [SignalKind::Breakout]);

    let (bars, mut volumes) = breakout_then(&[10.2, 9.9]);
    volumes.remove(BASE_DAYS);
    assert_eq!(failed_breakout_signal(&bars, &volumes), None);
    assert_eq!(failed_breakout_signal(&[], &[]), None);
}
