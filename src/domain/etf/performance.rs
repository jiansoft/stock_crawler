//! 以還原日線（除權息、分割已調整）計算 ETF 的含息期間報酬與近 1 年風險指標。
//!
//! - 期間報酬＝最新收盤 ÷ 期初收盤 − 1；期初取「基準日往前推該期間」當天或之前最後一個交易日。
//!   歷史不足（第一筆日線晚於期初目標日）時為 `None`。1 年以上換算成年化。
//! - 今年至今的期初是去年最後一個交易日。
//! - 波動、beta、夏普值用近 1 年的日報酬：波動＝標準差 × √252；beta＝與基準（0050）同日報酬的
//!   共變異數 ÷ 基準變異數；夏普值＝日報酬平均 × 252 ÷ 年化波動（未扣無風險利率）。
//! - 最大回撤取近 1 年收盤從前高到後低的最大跌幅。
//! - 淨值含息報酬（[`compute_nav_returns`]）用同一套期間規則，序列換成以現金配息還原的正式淨值。

use chrono::{Datelike, Months, NaiveDate};

use crate::domain::quote::technical::PriceBar;

/// 一檔 ETF 的含息報酬與風險指標；百分比皆以 % 表示（例如 12.5 代表 12.5%）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EtfPerformance {
    /// ETF 代號。
    pub stock_symbol: String,
    /// 計算基準日（最後一個有成交的交易日）。
    pub calc_date: Option<NaiveDate>,
    /// 1 週報酬。
    pub ret_1w: Option<f64>,
    /// 1 個月報酬。
    pub ret_1m: Option<f64>,
    /// 3 個月報酬。
    pub ret_3m: Option<f64>,
    /// 6 個月報酬。
    pub ret_6m: Option<f64>,
    /// 今年至今報酬。
    pub ret_ytd: Option<f64>,
    /// 1 年報酬。
    pub ret_1y: Option<f64>,
    /// 2 年年化報酬。
    pub ret_2y: Option<f64>,
    /// 3 年年化報酬。
    pub ret_3y: Option<f64>,
    /// 5 年年化報酬。
    pub ret_5y: Option<f64>,
    /// 10 年年化報酬。
    pub ret_10y: Option<f64>,
    /// 近 1 年年化波動。
    pub vol_1y: Option<f64>,
    /// 近 1 年最大回撤（負值）。
    pub mdd_1y: Option<f64>,
    /// 最大回撤的谷底日。
    pub mdd_date: Option<NaiveDate>,
    /// 近 1 年對基準的 beta。
    pub beta_1y: Option<f64>,
    /// 近 1 年夏普值。
    pub sharpe_1y: Option<f64>,
    /// 今年還原後最低收盤。
    pub year_low: Option<f64>,
    /// 今年還原後最高收盤。
    pub year_high: Option<f64>,
    /// 以正式淨值計算的含息報酬；沒有淨值歷史時為 `None`。
    pub nav_returns: Option<EtfNavReturns>,
}

/// 以正式淨值（現金配息已還原）計算的期間報酬；百分比，1 年以上為年化。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EtfNavReturns {
    /// 計算基準日（最後一筆淨值的日期）。
    pub calc_date: NaiveDate,
    /// 1 週報酬。
    pub ret_1w: Option<f64>,
    /// 1 個月報酬。
    pub ret_1m: Option<f64>,
    /// 3 個月報酬。
    pub ret_3m: Option<f64>,
    /// 6 個月報酬。
    pub ret_6m: Option<f64>,
    /// 今年至今報酬。
    pub ret_ytd: Option<f64>,
    /// 1 年報酬。
    pub ret_1y: Option<f64>,
    /// 2 年年化報酬。
    pub ret_2y: Option<f64>,
    /// 3 年年化報酬。
    pub ret_3y: Option<f64>,
    /// 5 年年化報酬。
    pub ret_5y: Option<f64>,
    /// 10 年年化報酬。
    pub ret_10y: Option<f64>,
}

/// 由還原後淨值序列（由舊到新，`close` 為淨值）計算期間報酬；序列為空時回傳 `None`。
pub fn compute_nav_returns(nav_bars: &[PriceBar]) -> Option<EtfNavReturns> {
    let perf = compute_performance("", nav_bars, &[]);
    Some(EtfNavReturns {
        calc_date: perf.calc_date?,
        ret_1w: perf.ret_1w,
        ret_1m: perf.ret_1m,
        ret_3m: perf.ret_3m,
        ret_6m: perf.ret_6m,
        ret_ytd: perf.ret_ytd,
        ret_1y: perf.ret_1y,
        ret_2y: perf.ret_2y,
        ret_3y: perf.ret_3y,
        ret_5y: perf.ret_5y,
        ret_10y: perf.ret_10y,
    })
}

/// 近 1 年至少要有這麼多個日報酬，才計算波動、beta 與夏普值（約半年）。
const MIN_RISK_SAMPLES: usize = 120;
const TRADING_DAYS: f64 = 252.0;

/// 計算一檔 ETF 的績效。`bars` 為還原後日線（由舊到新），`benchmark` 為基準 ETF 的還原後日線。
pub fn compute_performance(
    stock_symbol: &str,
    bars: &[PriceBar],
    benchmark: &[PriceBar],
) -> EtfPerformance {
    let mut perf = EtfPerformance {
        stock_symbol: stock_symbol.to_string(),
        ..Default::default()
    };
    let Some(last) = bars.last() else {
        return perf;
    };
    let as_of = last.date;
    perf.calc_date = Some(as_of);

    let months_back = |months: u32| as_of.checked_sub_months(Months::new(months));
    perf.ret_1w = as_of
        .checked_sub_days(chrono::Days::new(7))
        .and_then(|from| period_return(bars, from));
    perf.ret_1m = months_back(1).and_then(|from| period_return(bars, from));
    perf.ret_3m = months_back(3).and_then(|from| period_return(bars, from));
    perf.ret_6m = months_back(6).and_then(|from| period_return(bars, from));
    perf.ret_ytd = NaiveDate::from_ymd_opt(as_of.year() - 1, 12, 31)
        .and_then(|from| period_return(bars, from));
    perf.ret_1y = months_back(12).and_then(|from| period_return(bars, from));
    perf.ret_2y = annualized(bars, months_back(24), 2.0);
    perf.ret_3y = annualized(bars, months_back(36), 3.0);
    perf.ret_5y = annualized(bars, months_back(60), 5.0);
    perf.ret_10y = annualized(bars, months_back(120), 10.0);

    let year_start = NaiveDate::from_ymd_opt(as_of.year(), 1, 1);
    let this_year: Vec<f64> = bars
        .iter()
        .filter(|bar| Some(bar.date) >= year_start)
        .map(|bar| bar.close)
        .collect();
    perf.year_low = this_year.iter().copied().reduce(f64::min);
    perf.year_high = this_year.iter().copied().reduce(f64::max);

    if let Some(from) = months_back(12) {
        let window: Vec<&PriceBar> = bars.iter().filter(|bar| bar.date >= from).collect();
        if let Some((mdd, date)) = max_drawdown(&window) {
            perf.mdd_1y = Some(mdd);
            perf.mdd_date = Some(date);
        }
        let returns = daily_returns(&window);
        if returns.len() >= MIN_RISK_SAMPLES {
            let values: Vec<f64> = returns.iter().map(|(_, r)| *r).collect();
            let (mean, std) = mean_std(&values);
            let vol = std * TRADING_DAYS.sqrt();
            perf.vol_1y = Some(vol * 100.0);
            if vol > 0.0 {
                perf.sharpe_1y = Some(mean * TRADING_DAYS / vol);
            }
            let bench_window: Vec<&PriceBar> =
                benchmark.iter().filter(|bar| bar.date >= from).collect();
            perf.beta_1y = beta(&returns, &daily_returns(&bench_window));
        }
    }
    perf
}

/// 最新收盤相對 `from` 當天或之前最後一個交易日收盤的報酬（%）。
fn period_return(bars: &[PriceBar], from: NaiveDate) -> Option<f64> {
    let first = bars.first()?;
    if first.date > from {
        return None;
    }
    let base = bars.iter().rev().find(|bar| bar.date <= from)?;
    let last = bars.last()?;
    (base.close > 0.0).then(|| (last.close / base.close - 1.0) * 100.0)
}

fn annualized(bars: &[PriceBar], from: Option<NaiveDate>, years: f64) -> Option<f64> {
    let total = period_return(bars, from?)? / 100.0;
    (total > -1.0).then(|| ((1.0 + total).powf(1.0 / years) - 1.0) * 100.0)
}

/// 由前高到後低的最大跌幅（%，負值）與谷底日；不到兩筆或從未下跌時為 `None`。
fn max_drawdown(window: &[&PriceBar]) -> Option<(f64, NaiveDate)> {
    let mut peak = f64::MIN;
    let mut worst: Option<(f64, NaiveDate)> = None;
    for bar in window {
        peak = peak.max(bar.close);
        if peak <= 0.0 {
            continue;
        }
        let drawdown = (bar.close / peak - 1.0) * 100.0;
        if drawdown < worst.map_or(0.0, |(value, _)| value) {
            worst = Some((drawdown, bar.date));
        }
    }
    worst
}

fn daily_returns(window: &[&PriceBar]) -> Vec<(NaiveDate, f64)> {
    window
        .windows(2)
        .filter(|pair| pair[0].close > 0.0)
        .map(|pair| (pair[1].date, pair[1].close / pair[0].close - 1.0))
        .collect()
}

fn mean_std(values: &[f64]) -> (f64, f64) {
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
    (mean, variance.sqrt())
}

/// 以同一天都有報酬的日子計算 beta；共同樣本不足或基準沒有波動時為 `None`。
fn beta(returns: &[(NaiveDate, f64)], benchmark: &[(NaiveDate, f64)]) -> Option<f64> {
    let bench: std::collections::HashMap<NaiveDate, f64> = benchmark.iter().copied().collect();
    let pairs: Vec<(f64, f64)> = returns
        .iter()
        .filter_map(|(date, r)| bench.get(date).map(|b| (*r, *b)))
        .collect();
    if pairs.len() < MIN_RISK_SAMPLES {
        return None;
    }
    let n = pairs.len() as f64;
    let mean_r = pairs.iter().map(|(r, _)| r).sum::<f64>() / n;
    let mean_b = pairs.iter().map(|(_, b)| b).sum::<f64>() / n;
    let cov = pairs
        .iter()
        .map(|(r, b)| (r - mean_r) * (b - mean_b))
        .sum::<f64>();
    let var = pairs.iter().map(|(_, b)| (b - mean_b).powi(2)).sum::<f64>();
    (var > 0.0).then(|| cov / var)
}

#[cfg(test)]
mod tests {
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
}
