//! 52 週新高／新低與除權息還原。

use super::*;

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
