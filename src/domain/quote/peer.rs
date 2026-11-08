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
mod tests;
