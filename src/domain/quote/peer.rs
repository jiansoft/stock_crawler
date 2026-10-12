//! # 同業比較
//!
//! 同產業個股的各項指標中，算出產業中位數與目標股的名次。樣本由呼叫端挑好：
//! 本益比、股價淨值比 ≤ 0（虧損、未提供）不算樣本，其餘指標只排除沒有資料的股票。
//!
//! 名次以「比目標好的股票數 + 1」計算，同值同名次；本益比、股價淨值比越低越好，
//! 其餘指標（市值、殖利率、F 分數、毛利率、營收年增率）越高越好。

use rust_decimal::Decimal;

/// 指標的比較方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Better {
    /// 越低越好（本益比、股價淨值比）。
    Lower,
    /// 越高越好。
    Higher,
}

impl Better {
    /// API 使用的字串。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lower => "lower",
            Self::Higher => "higher",
        }
    }
}

/// 單一指標在同業間的位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerStanding {
    /// 產業中位數；沒有樣本時為 `None`。
    pub median: Option<Decimal>,
    /// 目標股的名次（1 起算）；目標沒有有效值時為 `None`。
    pub rank: Option<usize>,
    /// 有效樣本數（含目標股）。
    pub samples: usize,
}

/// 中位數：樣本數為偶數時取中間兩個的平均（與 PostgreSQL `percentile_cont(0.5)` 相同）。
pub fn median(values: &[Decimal]) -> Option<Decimal> {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let middle = sorted.len() / 2;
    match sorted.len() {
        0 => None,
        len if len % 2 == 1 => Some(sorted[middle]),
        _ => Some((sorted[middle - 1] + sorted[middle]) / Decimal::TWO),
    }
}

/// 目標值在樣本中的名次：比它好的樣本數 + 1，同值同名次。
pub fn rank(target: Decimal, values: &[Decimal], better: Better) -> usize {
    1 + values
        .iter()
        .filter(|value| match better {
            Better::Lower => **value < target,
            Better::Higher => **value > target,
        })
        .count()
}

/// 由全部同業的指標值（含目標股）算出中位數與目標名次。
///
/// `positive_only` 為真時只把 > 0 的值當樣本（本益比、股價淨值比）；目標值不是有效樣本時
/// 沒有名次。
pub fn standing(
    target: Option<Decimal>,
    values: &[Option<Decimal>],
    better: Better,
    positive_only: bool,
) -> PeerStanding {
    let valid = |value: &Decimal| !positive_only || *value > Decimal::ZERO;
    let samples: Vec<Decimal> = values.iter().flatten().copied().filter(valid).collect();
    PeerStanding {
        median: median(&samples),
        rank: target
            .filter(valid)
            .map(|target| rank(target, &samples, better)),
        samples: samples.len(),
    }
}

#[cfg(test)]
mod tests {
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
}
