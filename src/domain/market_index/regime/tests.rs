//! 加權指數趨勢狀態的均線、排列、站在均線同側天數、52 週高低點與穿越測試。

use super::*;

/// 由新到舊的收盤序列；日期從 2026-10-15 往前每天一筆（測試只看順序）。
fn series(closes: &[f64]) -> Vec<(NaiveDate, f64)> {
    closes
        .iter()
        .enumerate()
        .map(|(index, close)| (day(index), *close))
        .collect()
}

/// 2026-10-15 往前 `offset` 天。
fn day(offset: usize) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 15).expect("固定日期合法") - chrono::Days::new(offset as u64)
}

/// 均線：從 offset 起取 days 筆平均，資料不足為 None。
#[test]
fn moving_average_needs_enough_points() {
    let closes = [10.0, 20.0, 30.0, 40.0];
    assert_eq!(moving_average(&closes, 2, 0), Some(15.0));
    assert_eq!(moving_average(&closes, 2, 2), Some(35.0));
    assert_eq!(moving_average(&closes, 4, 1), None);
}

/// 排列：收盤與均線由高到低為多頭、由低到高為空頭，其餘或資料不足為糾結。
#[test]
fn alignment_classifies_order() {
    let all = |values: [f64; 4]| values.map(Some);
    assert_eq!(
        alignment(110.0, all([105.0, 100.0, 95.0, 90.0])),
        Alignment::Bullish
    );
    assert_eq!(
        alignment(80.0, all([85.0, 90.0, 95.0, 100.0])),
        Alignment::Bearish
    );
    assert_eq!(
        alignment(110.0, all([105.0, 100.0, 101.0, 90.0])),
        Alignment::Mixed
    );
    assert_eq!(
        alignment(110.0, [Some(105.0), Some(100.0), Some(95.0), None]),
        Alignment::Mixed
    );
}

/// 排列的 API 名稱與中文說明。
#[test]
fn alignment_names() {
    assert_eq!(
        [
            Alignment::Bullish.as_str(),
            Alignment::Bearish.as_str(),
            Alignment::Mixed.as_str()
        ],
        ["bullish", "bearish", "mixed"]
    );
    assert_eq!(
        [
            Alignment::Bullish.label(),
            Alignment::Bearish.label(),
            Alignment::Mixed.label()
        ],
        ["多頭排列", "空頭排列", "均線糾結"]
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

    assert_eq!(result.date, day(0));
    assert_eq!((result.close, result.change), (399.0, Some(1.0)));
    assert_eq!(result.change_percent, Some((399.0 / 398.0 - 1.0) * 100.0));
    assert_eq!(result.alignment, Alignment::Bullish);
    let ma20 = result.moving_average(20).expect("有 20 日均線");
    // 399～380 的平均 389.5，5 天前的均線 384.5：上揚。
    assert_eq!(ma20.value, Some(389.5));
    assert_eq!((ma20.above, ma20.rising), (Some(true), Some(true)));
    assert_eq!(ma20.distance_percent, Some((399.0 / 389.5 - 1.0) * 100.0));
    assert!(result.moving_average(10).is_none());
    // 一路上漲：300 天都在 60 日均線上方，但只有 241 天算得出 60 日均線。
    assert_eq!(result.days_on_side_of_ma60, Some(241));
    // 52 週取最近 250 筆：最高 399、最低 150。
    assert_eq!((result.high_52_week, result.low_52_week), (399.0, 150.0));
    assert_eq!(result.from_high_percent, 0.0);
    assert_eq!(result.from_low_percent, (399.0 / 150.0 - 1.0) * 100.0);
    assert_eq!(result.return_5d, Some((399.0 / 394.0 - 1.0) * 100.0));
    assert_eq!(result.return_20d, Some((399.0 / 379.0 - 1.0) * 100.0));
    assert_eq!(result.return_60d, Some((399.0 / 339.0 - 1.0) * 100.0));
    assert_eq!(result.return_240d, Some((399.0 / 159.0 - 1.0) * 100.0));
    assert!(regime(&[]).is_none());
}

/// 只有一天：沒有漲跌與均線，排列為糾結。
#[test]
fn regime_with_single_point() {
    let result = regime(&series(&[100.0])).expect("有資料");
    assert_eq!((result.change, result.change_percent), (None, None));
    assert_eq!(result.alignment, Alignment::Mixed);
    assert!(
        result
            .moving_averages
            .iter()
            .all(|status| status.value.is_none())
    );
    assert_eq!(result.days_on_side_of_ma60, None);
    assert_eq!((result.return_5d, result.return_20d), (None, None));
    assert_eq!((result.high_52_week, result.low_52_week), (100.0, 100.0));
}

/// 穿越：位置和前一天不同就算，由新到舊；只看最近 within 天，前一天沒有均線不算。
#[test]
fn crosses_detects_side_changes() {
    // 由新到舊；2 日均線：
    // 第 0 天 12 vs (12+5)/2=8.5 上方、第 1 天 5 vs (5+9)/2=7 下方 → 第 0 天站上。
    // 第 1 天下方、第 2 天 9 vs (9+8)/2=8.5 上方 → 第 1 天跌破。
    // 第 2 天上方、第 3 天 8 vs (8+8)/2=8 上方（含等於）→ 沒有穿越。
    let points = series(&[12.0, 5.0, 9.0, 8.0, 8.0]);
    let all = crosses(&points, 2, 10);
    assert_eq!(
        all,
        vec![
            MovingAverageCross {
                date: day(0),
                days: 2,
                above: true
            },
            MovingAverageCross {
                date: day(1),
                days: 2,
                above: false
            },
        ]
    );
    assert_eq!(crosses(&points, 2, 1), all[..1].to_vec());
    assert!(crosses(&points, 2, 0).is_empty());
    // 均線天數超過資料量就沒有穿越。
    assert!(crosses(&points, 5, 10).is_empty());
}

/// 讀取加權指數：分類固定 TAIEX、筆數為 LOOKBACK。
#[tokio::test]
async fn load_taiex_reads_lookback() {
    struct Fake;
    #[async_trait]
    impl IndexCloseQuery for Fake {
        async fn fetch_index_closes(
            &self,
            category: &str,
            until: Option<NaiveDate>,
            limit: usize,
        ) -> Result<Vec<(NaiveDate, f64)>> {
            assert_eq!((category, limit), (TAIEX, LOOKBACK));
            Ok(until.map(|date| vec![(date, 1.0)]).unwrap_or_default())
        }
    }
    assert_eq!(
        load_taiex(&Fake, Some(day(0))).await.expect("讀取成功"),
        vec![(day(0), 1.0)]
    );
    assert!(load_taiex(&Fake, None).await.expect("讀取成功").is_empty());
}
