//! 全市場填息基準：依現金殖利率分組的填息率與填息天數的測試。

use rust_decimal_macros::dec;

use super::*;

fn sample(bucket: YieldBucket, days: Option<u32>, observed: u32) -> FillSample {
    FillSample {
        bucket,
        trading_days_to_fill: days,
        trading_days_observed: observed,
    }
}

/// 分組邊界：3%、5%、7% 歸到較高的一組；沒有現金股利或收盤為 0 時不分組。
#[test]
fn yield_bucket_boundaries() {
    assert_eq!(
        YieldBucket::of(dec!(2.99), dec!(100)),
        Some(YieldBucket::Below3)
    );
    assert_eq!(
        YieldBucket::of(dec!(3), dec!(100)),
        Some(YieldBucket::From3To5)
    );
    assert_eq!(
        YieldBucket::of(dec!(5), dec!(100)),
        Some(YieldBucket::From5To7)
    );
    assert_eq!(
        YieldBucket::of(dec!(7), dec!(100)),
        Some(YieldBucket::AtLeast7)
    );
    assert_eq!(YieldBucket::of(Decimal::ZERO, dec!(100)), None);
    assert_eq!(YieldBucket::of(dec!(1), Decimal::ZERO), None);
}

/// 填息率與中位數只看觀察期已滿的事件；未滿的只算進 events。
#[test]
fn bucket_stats_uses_mature_events_only() {
    let samples = [
        sample(YieldBucket::Below3, Some(1), 120),
        sample(YieldBucket::Below3, Some(30), 120),
        sample(YieldBucket::Below3, Some(90), 120),
        sample(YieldBucket::Below3, None, 120),
        sample(YieldBucket::Below3, None, 15),
        sample(YieldBucket::Below3, Some(2), 5),
        sample(YieldBucket::AtLeast7, None, 120),
    ];
    let stats = bucket_stats(&samples);
    assert_eq!(
        stats.iter().map(|s| s.bucket).collect::<Vec<_>>(),
        YieldBucket::ALL
    );
    assert_eq!(
        stats[0],
        BucketStats {
            bucket: YieldBucket::Below3,
            events: 6,
            mature_events: 4,
            fill_rate_20: Some(25.0),
            fill_rate_60: Some(50.0),
            fill_rate_120: Some(75.0),
            median_trading_days: Some(30.0),
        }
    );
    let high = stats[3];
    assert_eq!((high.mature_events, high.fill_rate_120), (1, Some(0.0)));
    assert_eq!(high.median_trading_days, None, "沒有已填息者");
}

/// 5 年期間與持股填息的近 5 年歷史一致（5 × 365 + 1 天）。
#[test]
fn stats_window_spans_whole_years() {
    let today = NaiveDate::from_ymd_opt(2026, 10, 31).expect("日期應合法");
    assert_eq!(
        stats_window(today, 5),
        (
            NaiveDate::from_ymd_opt(2021, 10, 31).expect("日期應合法"),
            today
        )
    );
}

/// 沒有樣本的組次數為 0、比例與中位數為 None；偶數筆中位數取中間兩筆平均。
#[test]
fn bucket_stats_handles_empty_and_even_counts() {
    let stats = bucket_stats(&[
        sample(YieldBucket::From3To5, Some(4), 120),
        sample(YieldBucket::From3To5, Some(10), 120),
    ]);
    assert_eq!(stats[1].median_trading_days, Some(7.0));
    let empty = stats[2];
    assert_eq!(
        (empty.events, empty.fill_rate_20, empty.median_trading_days),
        (0, None, None)
    );
    assert_eq!(YieldBucket::AtLeast7.code(), "at_least_7");
    assert_eq!(YieldBucket::From5To7.label(), "5～7%");
}
