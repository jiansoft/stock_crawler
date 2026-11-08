//! 市場估值水位：上市櫃官方本益比與股價淨值比的每日中位數的測試。

use super::*;

fn day(d: u32, pe: Option<f64>, samples: i64) -> MarketValuationDay {
    MarketValuationDay {
        date: NaiveDate::from_ymd_opt(2026, 1, d).expect("合法日期"),
        median_pe: pe,
        median_pb: None,
        pe_samples: samples,
        pb_samples: samples,
    }
}

/// 分位數線性內插：與 percentile_cont 相同。
#[test]
fn quantile_interpolates_linearly() {
    let values = [10.0, 12.0, 14.0, 20.0];
    assert_eq!(quantile(&values, 0.0), 10.0);
    assert_eq!(quantile(&values, 0.5), 13.0);
    assert_eq!(quantile(&values, 0.25), 11.5);
    assert_eq!(quantile(&values, 1.0), 20.0);
    assert_eq!(quantile(&[7.0], 0.75), 7.0);
}

/// 百分位是不高於最新一天的日子占比；沒有最新值或沒有資料時不算。
#[test]
fn distribution_reports_quartiles_and_percentile() {
    let stats = distribution(vec![14.0, 10.0, 20.0, 12.0], Some(14.0)).expect("有資料");
    assert_eq!(
        stats,
        ValuationDistribution {
            current: 14.0,
            percentile: 75.0,
            min: 10.0,
            p25: 11.5,
            median: 13.0,
            p75: 15.5,
            max: 20.0,
            days: 4,
        }
    );
    assert!(distribution(vec![10.0], None).is_none());
    assert!(distribution(Vec::new(), Some(10.0)).is_none());
}

/// 樣本數不到典型值一半的日子被去掉；空資料原樣回傳。
#[test]
fn complete_days_drops_partial_days() {
    let kept = complete_days(vec![
        day(2, Some(15.0), 1_000),
        day(5, Some(15.5), 990),
        day(6, Some(40.0), 30),
    ]);
    assert_eq!(kept.len(), 2);
    assert!(kept.iter().all(|day| day.pe_samples >= 990));
    assert!(complete_days(Vec::new()).is_empty());
}
