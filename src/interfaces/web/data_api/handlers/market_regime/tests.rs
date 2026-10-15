//! 大盤趨勢狀態的均線、排列、站在均線同側天數與 52 週高低點測試。

use super::*;

/// 由新到舊的收盤序列；日期從 2026-10-15 往前每天一筆（測試只看順序）。
fn series(closes: &[f64]) -> Vec<(NaiveDate, f64)> {
    let latest = NaiveDate::from_ymd_opt(2026, 10, 15).expect("固定日期合法");
    closes
        .iter()
        .enumerate()
        .map(|(index, close)| (latest - chrono::Days::new(index as u64), *close))
        .collect()
}

/// 均線：從 offset 起取 days 筆平均，資料不足為 None。
#[test]
fn moving_average_needs_enough_points() {
    let closes = [10.0, 20.0, 30.0, 40.0];
    assert_eq!(moving_average(&closes, 2, 0), Some(15.0));
    assert_eq!(moving_average(&closes, 2, 2), Some(35.0));
    assert_eq!(moving_average(&closes, 4, 1), None);
}

/// 排列：收盤與均線由高到低為多頭、由低到高為空頭，其餘或資料不足為混合。
#[test]
fn alignment_classifies_order() {
    let all = |values: [f64; 4]| values.map(Some);
    assert_eq!(alignment(110.0, all([105.0, 100.0, 95.0, 90.0])), "bullish");
    assert_eq!(alignment(80.0, all([85.0, 90.0, 95.0, 100.0])), "bearish");
    assert_eq!(alignment(110.0, all([105.0, 100.0, 101.0, 90.0])), "mixed");
    assert_eq!(
        alignment(110.0, [Some(105.0), Some(100.0), Some(95.0), None]),
        "mixed"
    );
}

/// 站在均線同一側的天數：數到第一次換邊為止；最新一天沒有均線為 None。
#[test]
fn days_on_side_counts_until_cross() {
    // 2 日均線：第 0、1 天收盤高於均線，第 2 天（5 < 均線 7）換邊。
    let closes = [12.0, 11.0, 5.0, 9.0, 9.0];
    assert_eq!(days_on_side(&closes, 2), Some(2));
    // 資料不足時只數到有均線的日子。
    assert_eq!(days_on_side(&[3.0, 2.0, 1.0], 2), Some(2));
    assert_eq!(days_on_side(&[3.0], 2), None);
}

/// 整體：漲跌、均線位置與方向、52 週高低點、近 N 日漲跌幅。
#[test]
fn regime_summarizes_taiex() {
    // 最新 300 天由 100 線性漲到 399（由新到舊為 399、398、…、100）。
    let closes: Vec<f64> = (0..300).map(|index| 399.0 - f64::from(index)).collect();
    let result = regime(&series(&closes)).expect("有資料");

    assert_eq!(result.data_as_of, "2026-10-15");
    assert_eq!((result.close, result.change), (399.0, Some(1.0)));
    assert_eq!(result.change_percent, Some(round2(1.0 / 398.0 * 100.0)));
    assert_eq!(result.alignment, "bullish");
    let ma20 = &result.moving_averages[1];
    assert_eq!(ma20.days, 20);
    // 399～380 的平均 389.5，5 天前的均線 384.5：上揚。
    assert_eq!(ma20.value, Some(389.5));
    assert_eq!((ma20.above, ma20.rising), (Some(true), Some(true)));
    assert_eq!(
        ma20.distance_percent,
        Some(round2((399.0 / 389.5 - 1.0) * 100.0))
    );
    // 一路上漲：300 天都在 60 日均線上方，但只有 241 天算得出 60 日均線。
    assert_eq!(result.days_on_side_of_ma60, Some(241));
    // 52 週取最近 250 筆：最高 399、最低 150。
    assert_eq!((result.high_52_week, result.low_52_week), (399.0, 150.0));
    assert_eq!(result.from_high_percent, 0.0);
    assert_eq!(
        result.from_low_percent,
        round2((399.0 / 150.0 - 1.0) * 100.0)
    );
    assert_eq!(
        result.return_20d,
        Some(round2((399.0 / 379.0 - 1.0) * 100.0))
    );
    assert_eq!(
        result.return_240d,
        Some(round2((399.0 / 159.0 - 1.0) * 100.0))
    );
    assert!(regime(&[]).is_none());
}

/// 只有一天：沒有漲跌與均線，排列為混合。
#[test]
fn regime_with_single_point() {
    let result = regime(&series(&[100.0])).expect("有資料");
    assert_eq!((result.change, result.change_percent), (None, None));
    assert_eq!(result.alignment, "mixed");
    assert!(
        result
            .moving_averages
            .iter()
            .all(|status| status.value.is_none())
    );
    assert_eq!(result.days_on_side_of_ma60, None);
    assert_eq!(result.return_20d, None);
    assert_eq!((result.high_52_week, result.low_52_week), (100.0, 100.0));
}
