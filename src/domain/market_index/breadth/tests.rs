//! 市場廣度：上市櫃合併站上 20／60／240 日均線的股票比例與創 52 週新高／新低的家數的測試。

use super::*;

fn day(d: u32, above_ma60: i32, new_high: i32, new_low: i32) -> BreadthDay {
    BreadthDay {
        date: NaiveDate::from_ymd_opt(2026, 10, d).expect("合法日期"),
        above_ma20: 30,
        below_ma20: 70,
        above_ma60,
        below_ma60: 100 - above_ma60,
        above_ma240: 0,
        below_ma240: 0,
        new_high,
        new_low,
    }
}

/// 資料日比例、`window` 日前比例、近 `window` 日新高新低加總；沒有家數的比例為 `None`。
#[test]
fn summarize_latest_and_window() {
    let days = [
        day(16, 40, 1, 5),
        day(15, 45, 2, 4),
        day(14, 50, 3, 3),
        day(13, 55, 4, 2),
        day(12, 60, 5, 1),
        day(9, 65, 100, 100),
    ];
    let breadth = summarize(&days, 5).expect("有資料");
    assert_eq!(breadth.date, days[0].date);
    assert_eq!(breadth.above_ma20_percent, Some(30.0));
    assert_eq!(breadth.above_ma60_percent, Some(40.0));
    assert_eq!(breadth.above_ma240_percent, None);
    assert_eq!(breadth.above_ma60_percent_before, Some(65.0));
    // 第 6 天（10-09）只當比較基準，不算進新高新低。
    assert_eq!(
        (breadth.new_highs, breadth.new_lows, breadth.days),
        (15, 15, 5)
    );

    // 資料不足：沒有 5 日前、只加總現有的天數。
    let short = summarize(&days[..2], 5).expect("有資料");
    assert_eq!(short.above_ma60_percent_before, None);
    assert_eq!((short.new_highs, short.days), (3, 2));
    assert!(summarize(&[], 5).is_none());
}
