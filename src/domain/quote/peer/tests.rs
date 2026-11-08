//! 同業比較的測試。

use rust_decimal_macros::dec;

use super::*;

/// 奇數取中間、偶數取中間兩個的平均、空集合沒有中位數。
#[test]
fn median_matches_percentile_cont() {
    assert_eq!(median(&[dec!(3), dec!(1), dec!(2)]), Some(dec!(2)));
    assert_eq!(
        median(&[dec!(4), dec!(1), dec!(2), dec!(3)]),
        Some(dec!(2.5))
    );
    assert_eq!(median(&[]), None);
}

/// 名次依方向計算，同值同名次。
#[test]
fn rank_follows_direction_and_ties() {
    let values = [dec!(10), dec!(20), dec!(20), dec!(30)];
    assert_eq!(rank(dec!(10), &values, Better::Lower), 1);
    assert_eq!(rank(dec!(20), &values, Better::Lower), 2);
    assert_eq!(rank(dec!(20), &values, Better::Higher), 2);
    assert_eq!(rank(dec!(30), &values, Better::Higher), 1);
    assert_eq!(Better::Lower.as_str(), "lower");
    assert_eq!(Better::Higher.as_str(), "higher");
}

/// 只取正值時，虧損（0）不算樣本，目標虧損就沒有名次；沒有資料的股票一律不算。
#[test]
fn standing_skips_missing_and_non_positive_values() {
    let values = [
        Some(dec!(12)),
        Some(dec!(0)),
        None,
        Some(dec!(18)),
        Some(dec!(15)),
    ];
    let pe = standing(Some(dec!(15)), &values, Better::Lower, true);
    assert_eq!(
        pe,
        PeerStanding {
            median: Some(dec!(15)),
            rank: Some(2),
            samples: 3
        }
    );
    let loss = standing(Some(dec!(0)), &values, Better::Lower, true);
    assert_eq!((loss.rank, loss.samples), (None, 3));

    let growth = standing(Some(dec!(0)), &values, Better::Higher, false);
    assert_eq!((growth.rank, growth.samples), (Some(4), 4));
    assert_eq!(standing(None, &[], Better::Higher, false).median, None);
}
