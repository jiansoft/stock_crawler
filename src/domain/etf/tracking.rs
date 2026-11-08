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
mod tests;
