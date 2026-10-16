//! 大盤趨勢狀態回應的轉換測試；計算本身的測試在 `domain::market_index::regime`。

use chrono::NaiveDate;

use super::*;

/// 由新到舊的收盤序列；日期從 2026-10-15 往前每天一筆。
fn series(closes: &[f64]) -> Vec<(NaiveDate, f64)> {
    let latest = NaiveDate::from_ymd_opt(2026, 10, 15).expect("固定日期合法");
    closes
        .iter()
        .enumerate()
        .map(|(index, close)| (latest - chrono::Days::new(index as u64), *close))
        .collect()
}

/// 回應：日期字串、排列名稱、數值四捨五入到兩位。
#[test]
fn response_rounds_values() {
    // 最新 300 天由 100 線性漲到 399（由新到舊為 399、398、…、100）。
    let closes: Vec<f64> = (0..300).map(|index| 399.0 - f64::from(index)).collect();
    let state = regime::regime(&series(&closes)).expect("有資料");
    let result = response(&state);

    assert_eq!(result.data_as_of, "2026-10-15");
    assert_eq!((result.close, result.change), (399.0, Some(1.0)));
    assert_eq!(result.change_percent, Some(round2(1.0 / 398.0 * 100.0)));
    assert_eq!(result.alignment, "bullish");
    let ma20 = &result.moving_averages[1];
    assert_eq!((ma20.days, ma20.value), (20, Some(389.5)));
    assert_eq!((ma20.above, ma20.rising), (Some(true), Some(true)));
    assert_eq!(ma20.distance_percent, Some(2.44));
    assert_eq!(result.days_on_side_of_ma60, Some(241));
    assert_eq!((result.high_52_week, result.low_52_week), (399.0, 150.0));
    assert_eq!(
        (result.from_high_percent, result.from_low_percent),
        (0.0, 166.0)
    );
    assert_eq!(result.return_20d, Some(5.28));
    assert_eq!(result.return_60d, Some(17.7));
    assert_eq!(result.return_240d, Some(150.94));
}

/// 只有一天：沒有漲跌、均線與近期漲跌幅。
#[test]
fn response_with_single_point() {
    let state = regime::regime(&series(&[100.0])).expect("有資料");
    let result = response(&state);
    assert_eq!((result.change, result.change_percent), (None, None));
    assert_eq!(result.alignment, "mixed");
    assert!(
        result
            .moving_averages
            .iter()
            .all(|status| status.value.is_none() && status.above.is_none())
    );
    assert_eq!(
        (result.return_20d, result.return_60d, result.return_240d),
        (None, None, None)
    );
}
