//! 指定基準的年化追蹤誤差；比較相同起訖日期的含息日報酬，避免缺日造成錯配。
use crate::domain::quote::technical::PriceBar;
use chrono::{Months, NaiveDate};

/// 近一年同區間日報酬差的樣本統計；數值以百分比表示。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrackingStats {
    pub samples: usize,
    pub tracking_error: Option<f64>,
    pub tracking_difference: Option<f64>,
}

/// 至少 120 組同區間報酬才計算年化樣本標準差及累積報酬差。
pub fn compute(bars: &[PriceBar], benchmark: &[PriceBar]) -> TrackingStats {
    let Some(last) = bars.last() else {
        return TrackingStats::default();
    };
    let from = last
        .date
        .checked_sub_months(Months::new(12))
        .unwrap_or(NaiveDate::MIN);
    let returns = |series: &[PriceBar]| -> std::collections::BTreeMap<(NaiveDate, NaiveDate), f64> {
        series
            .windows(2)
            .filter_map(|pair| {
                let (a, b) = (&pair[0], &pair[1]);
                (b.date >= from
                    && b.date <= last.date
                    && a.date < b.date
                    && a.close > 0.0
                    && b.close > 0.0
                    && a.close.is_finite()
                    && b.close.is_finite())
                .then(|| ((a.date, b.date), b.close / a.close - 1.0))
            })
            .collect()
    };
    let bench = returns(benchmark);
    let pairs: Vec<_> = returns(bars)
        .into_iter()
        .filter_map(|(dates, r)| bench.get(&dates).map(|b| (r, *b)))
        .collect();
    let n = pairs.len();
    if n < 120 {
        return TrackingStats {
            samples: n,
            ..Default::default()
        };
    }
    let mean = pairs.iter().map(|(r, b)| r - b).sum::<f64>() / n as f64;
    let variance = pairs
        .iter()
        .map(|(r, b)| (r - b - mean).powi(2))
        .sum::<f64>()
        / (n - 1) as f64;
    let fund = pairs.iter().fold(1.0, |total, (r, _)| total * (1.0 + r));
    let baseline = pairs.iter().fold(1.0, |total, (_, b)| total * (1.0 + b));
    TrackingStats {
        samples: n,
        tracking_error: Some(variance.sqrt() * 252.0_f64.sqrt() * 100.0),
        tracking_difference: Some((fund - baseline) * 100.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identical_returns_have_zero_error_and_missing_intervals_are_not_mismatched() {
        let first = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let series: Vec<_> = (0..160)
            .map(|i| PriceBar {
                date: first + chrono::Days::new(i),
                close: 100.0 + (i * i) as f64 / 100.0,
                high: 100.0,
                low: 100.0,
            })
            .collect();
        let result = compute(&series, &series);
        assert_eq!(result.samples, 159);
        assert_eq!(result.tracking_error, Some(0.0));
        let mut missing = series.clone();
        missing.remove(40);
        assert_eq!(compute(&series, &missing).samples, 157);
        assert_eq!(compute(&series[..30], &series).tracking_error, None);
        let changed: Vec<_> = series
            .iter()
            .enumerate()
            .map(|(i, b)| PriceBar {
                close: b.close * (1.0 + 0.0001 * (i % 2) as f64),
                ..*b
            })
            .collect();
        assert!(compute(&changed, &series).tracking_error.unwrap() > 0.0);
    }

    #[test]
    fn minimum_sample_and_sample_standard_deviation_match_known_returns() {
        let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let mut close = 100.0;
        let fund: Vec<_> = (0..121)
            .map(|i| {
                if i > 0 {
                    close *= if i % 2 == 0 { 0.99 } else { 1.01 };
                }
                PriceBar {
                    date: date + chrono::Days::new(i),
                    close,
                    high: close,
                    low: close,
                }
            })
            .collect();
        let benchmark: Vec<_> = fund
            .iter()
            .map(|bar| PriceBar {
                close: 100.0,
                ..*bar
            })
            .collect();
        assert_eq!(compute(&fund[..120], &benchmark).tracking_error, None);
        let actual = compute(&fund, &benchmark);
        let expected = (120.0_f64 * 0.01_f64.powi(2) / 119.0 * 252.0).sqrt() * 100.0;
        assert!((actual.tracking_error.unwrap() - expected).abs() < 1e-10);
        assert!(
            (actual.tracking_difference.unwrap() - ((1.01_f64 * 0.99).powi(60) - 1.0) * 100.0)
                .abs()
                < 1e-10
        );
        assert_eq!(compute(&[], &benchmark), TrackingStats::default());
        assert_eq!(compute(&fund, &[]).samples, 0);
    }
}
