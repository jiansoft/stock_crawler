//! 本益比／股價淨值比歷史區間的測試。

use rust_decimal_macros::dec;

use super::*;

/// 分位數與 PostgreSQL percentile_cont 相同：10、20、30、40、50 的 P25＝20、中位數＝30。
#[test]
fn quantiles_match_percentile_cont() {
    let history = [dec!(50), dec!(10), dec!(30), dec!(20), dec!(40)];
    let band = summarize(&history, Some(dec!(30)), Some(dec!(600))).expect("有樣本");
    assert_eq!((band.min, band.p25, band.median), (10.0, 20.0, 30.0));
    assert_eq!((band.p75, band.max), (40.0, 50.0));
    // 小於等於 30 的有 3 筆／5 筆。
    assert_eq!(band.percentile, Some(60.0));
    assert_eq!(
        band.implied_prices,
        Some(ImpliedPrices {
            p25: 400.0,
            median: 600.0,
            p75: 800.0
        })
    );

    // 偶數筆要內插：1、2、3、4 的中位數 2.5、P25 1.75。
    let even = summarize(&[dec!(4), dec!(1), dec!(3), dec!(2)], None, None).expect("有樣本");
    assert_eq!((even.p25, even.median, even.p75), (1.75, 2.5, 3.25));
}

/// 0 與負數（虧損、未提供）不算樣本，另計排除天數；目前值為 0 時沒有百分位與對應股價。
#[test]
fn non_positive_ratios_are_excluded() {
    let history = [dec!(0), dec!(-5), dec!(12), dec!(18)];
    let band = summarize(&history, Some(dec!(0)), Some(dec!(100))).expect("有樣本");
    assert_eq!((band.samples, band.excluded), (2, 2));
    assert_eq!(band.current, None);
    assert_eq!(band.percentile, None);
    assert_eq!(band.implied_prices, None);

    assert_eq!(summarize(&[dec!(0), dec!(-1)], Some(dec!(3)), None), None);
    assert_eq!(summarize(&[], None, None), None);
}

/// 目前值落在最低或最高；沒有收盤價時不算對應股價。
#[test]
fn percentile_at_extremes_and_missing_price() {
    let history = [dec!(8), dec!(9), dec!(10)];
    let low = summarize(&history, Some(dec!(7)), None).expect("有樣本");
    assert_eq!(low.percentile, Some(0.0));
    assert_eq!(low.implied_prices, None);
    let high = summarize(&history, Some(dec!(10)), Some(dec!(0))).expect("有樣本");
    assert_eq!(high.percentile, Some(100.0));
    assert_eq!(high.implied_prices, None);

    let single = summarize(&[dec!(15)], Some(dec!(15)), Some(dec!(45))).expect("有樣本");
    assert_eq!((single.p25, single.median, single.p75), (15.0, 15.0, 15.0));
    assert_eq!(single.percentile, Some(100.0));
}
