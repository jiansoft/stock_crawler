//! 以還原日線（除權息、分割已調整）計算 ETF 的含息期間報酬與近 1 年風險指標的測試。

use super::*;

fn bar(date: NaiveDate, close: f64) -> PriceBar {
    PriceBar {
        date,
        close,
        high: close,
        low: close,
    }
}

/// 從 `start` 起每天一筆（含週末，簡化），收盤由 `price(i)` 決定。
fn series(start: NaiveDate, days: u64, price: impl Fn(u64) -> f64) -> Vec<PriceBar> {
    (0..days)
        .map(|i| bar(start + chrono::Days::new(i), price(i)))
        .collect()
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("固定日期合法")
}

#[test]
fn nav_returns_follow_price_period_rules() {
    assert_eq!(compute_nav_returns(&[]), None);
    // 兩年每天漲一點：1 年報酬與 2 年年化都有，3 年以上歷史不足。
    let bars = series(date(2024, 10, 1), 731, |i| 100.0 + i as f64 * 0.1);
    let nav = compute_nav_returns(&bars).expect("有淨值");
    assert_eq!(nav.calc_date, date(2026, 10, 1));
    let price = compute_performance("x", &bars, &[]);
    assert_eq!(nav.ret_1y, price.ret_1y);
    assert_eq!(nav.ret_2y, price.ret_2y);
    assert!(nav.ret_1m.is_some() && nav.ret_ytd.is_some());
    assert_eq!(nav.ret_3y, None);
}

#[test]
fn empty_bars_yield_empty_performance() {
    let perf = compute_performance("0050", &[], &[]);
    assert_eq!(perf.stock_symbol, "0050");
    assert_eq!(perf.calc_date, None);
    assert_eq!(perf.ret_1y, None);
}

#[test]
fn period_returns_use_the_last_bar_on_or_before_the_target() {
    // 2024-01-01 起每天 +0.1 元，到 2026-10-21。
    let start = date(2024, 1, 1);
    let days = (date(2026, 10, 21) - start).num_days() as u64 + 1;
    let bars = series(start, days, |i| 100.0 + i as f64 * 0.1);
    let perf = compute_performance("X", &bars, &[]);
    let last = bars.last().expect("有資料").close;
    let close_on = |d: NaiveDate| bars.iter().find(|b| b.date == d).expect("有該日").close;
    let pct = |base: f64| (last / base - 1.0) * 100.0;

    assert_eq!(perf.calc_date, Some(date(2026, 10, 21)));
    assert!((perf.ret_1w.unwrap() - pct(close_on(date(2026, 10, 14)))).abs() < 1e-9);
    assert!((perf.ret_1m.unwrap() - pct(close_on(date(2026, 9, 21)))).abs() < 1e-9);
    assert!((perf.ret_ytd.unwrap() - pct(close_on(date(2025, 12, 31)))).abs() < 1e-9);
    assert!((perf.ret_1y.unwrap() - pct(close_on(date(2025, 10, 21)))).abs() < 1e-9);
    let two_year = pct(close_on(date(2024, 10, 21))) / 100.0;
    assert!((perf.ret_2y.unwrap() - ((1.0 + two_year).sqrt() - 1.0) * 100.0).abs() < 1e-9);
    // 歷史不足 3 年。
    assert_eq!(perf.ret_3y, None);
    assert_eq!(perf.ret_10y, None);
    assert_eq!(perf.year_low, Some(close_on(date(2026, 1, 1))));
    assert_eq!(perf.year_high, Some(last));
    // 一路上漲：沒有回撤，波動為 0 時沒有夏普值。
    assert_eq!(perf.mdd_1y, None);
}

#[test]
fn drawdown_and_risk_metrics_follow_the_last_year() {
    let start = date(2025, 1, 1);
    // 前 200 天漲到 120，接著跌到 90，再回到 110；中間夾一點日內擺盪產生波動。
    let bars = series(start, 400, |i| {
        let base = if i < 200 {
            100.0 + i as f64 * 0.1
        } else if i < 260 {
            120.0 - (i - 200) as f64 * 0.5
        } else {
            90.0 + (i - 260) as f64 * (20.0 / 140.0)
        };
        base * if i % 2 == 0 { 1.01 } else { 0.99 }
    });
    // 基準與自己同步擺盪，振幅一半，beta 應約為 2。
    let benchmark = series(start, 400, |i| {
        100.0 * if i % 2 == 0 { 1.005 } else { 0.995 }
    });
    let perf = compute_performance("X", &bars, &benchmark);

    let (mdd, trough) = (
        perf.mdd_1y.expect("有回撤"),
        perf.mdd_date.expect("有谷底日"),
    );
    assert!(mdd < -20.0, "跌幅約 25%：{mdd}");
    assert_eq!(trough, start + chrono::Days::new(261));
    assert!(perf.vol_1y.expect("有波動") > 0.0);
    assert!(perf.sharpe_1y.is_some());
    let beta = perf.beta_1y.expect("有 beta");
    assert!((beta - 2.0).abs() < 0.3, "beta 約 2：{beta}");
}

#[test]
fn risk_metrics_need_enough_samples() {
    let bars = series(date(2026, 8, 1), 60, |i| 100.0 + (i % 3) as f64);
    let perf = compute_performance("X", &bars, &bars);
    assert_eq!(perf.vol_1y, None);
    assert_eq!(perf.beta_1y, None);
    assert_eq!(perf.ret_1y, None);
    assert!(perf.ret_1w.is_some());
}
