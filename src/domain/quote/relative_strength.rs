//! 相對強度（RS）：近 60／120／240 日還原報酬加權後在上市櫃全體中的百分位分數。
//!
//! 加權報酬＝0.4 × 近 60 日 ＋ 0.3 × 近 120 日 ＋ 0.3 × 近 240 日（近 N 日報酬取
//! [`super::forward_return`] 基準日 T−N 的 N 日欄位），分數 1–99，99 為最強；分數、名次一律以
//! 當天上市櫃全體計算。本模組只放純計算與查詢介面，SQL 在 infra。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

/// 穿越時要標記的分數門檻：80 為強勢、50 為中位（高門檻在前）。
pub const THRESHOLDS: [u8; 2] = [80, 50];

/// 一檔股票某個交易日的相對強度。
#[derive(Debug, Clone, PartialEq)]
pub struct RelativeStrengthDay {
    /// 股票代號。
    pub stock_symbol: String,
    /// 交易日。
    pub date: NaiveDate,
    /// 加權報酬（%）。
    pub weighted: f64,
    /// 加權報酬比它低的家數。
    pub weaker: i64,
    /// 名次（比它強的家數＋1，同分同名次）。
    pub rank: i64,
    /// 當天計分的股票總數。
    pub universe: i64,
}

impl RelativeStrengthDay {
    /// 相對強度分數 1–99。
    pub fn rs_score(&self) -> u8 {
        percentile_score(
            usize::try_from(self.weaker).unwrap_or(0),
            usize::try_from(self.universe).unwrap_or(0),
        )
    }
}

/// 比它弱的家數 `weaker` 在全體 `total` 中的百分位，換算成 1–99 分（只有一檔時為 99）。
pub fn percentile_score(weaker: usize, total: usize) -> u8 {
    if total <= 1 {
        return 99;
    }
    let rank = weaker as f64 / (total - 1) as f64;
    // rank 在 0–1 之間，換算後必在 1–99。
    (rank * 98.0).floor() as u8 + 1
}

/// 穿越門檻的方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossDirection {
    /// 由門檻下方升到門檻以上（含）。
    Up,
    /// 由門檻以上（含）跌到門檻下方。
    Down,
}

impl CrossDirection {
    /// API 使用的名稱：`up` 或 `down`。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

/// 前一個交易日分數 `previous` 到當天分數 `current` 穿越的 [`THRESHOLDS`]，高門檻在前。
pub fn crossings(previous: u8, current: u8) -> Vec<(u8, CrossDirection)> {
    THRESHOLDS
        .into_iter()
        .filter_map(|threshold| {
            match (previous >= threshold, current >= threshold) {
                (false, true) => Some(CrossDirection::Up),
                (true, false) => Some(CrossDirection::Down),
                _ => None,
            }
            .map(|direction| (threshold, direction))
        })
        .collect()
}

/// 相對強度歷史的查詢介面。
#[async_trait]
pub trait RelativeStrengthQuery: Send + Sync {
    /// `until`（含）之前最近 `days` 個交易日，每天以上市櫃全體計分後取 `stock_symbols` 的分數，
    /// 依代號、再由新到舊排列。報酬還沒回填的交易日沒有資料；上市未滿 240 個交易日、暫停交易
    /// 或非上市櫃的股票沒有分數。
    async fn fetch_relative_strength(
        &self,
        until: NaiveDate,
        days: u16,
        stock_symbols: &[String],
    ) -> Result<Vec<RelativeStrengthDay>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 百分位分數：最弱 1、最強 99、只有一檔 99；`rs_score` 以 weaker／universe 換算。
    #[test]
    fn percentile_score_maps_to_1_99() {
        assert_eq!(percentile_score(0, 5), 1);
        assert_eq!(percentile_score(4, 5), 99);
        assert_eq!(percentile_score(2, 5), 50);
        assert_eq!(percentile_score(0, 1), 99);
        let day = RelativeStrengthDay {
            stock_symbol: "2330".to_owned(),
            date: NaiveDate::from_ymd_opt(2026, 10, 13).expect("合法日期"),
            weighted: 1.0,
            weaker: 50,
            rank: 51,
            universe: 101,
        };
        assert_eq!(day.rs_score(), 50);
    }

    /// 穿越：同一天跨兩個門檻兩筆都列（高門檻在前）；剛好等於門檻算在門檻以上；沒穿越為空。
    #[test]
    fn crossings_detect_thresholds() {
        assert_eq!(crossings(79, 80), [(80, CrossDirection::Up)]);
        assert_eq!(crossings(80, 79), [(80, CrossDirection::Down)]);
        assert_eq!(
            crossings(85, 40),
            [(80, CrossDirection::Down), (50, CrossDirection::Down)]
        );
        assert_eq!(crossings(49, 50), [(50, CrossDirection::Up)]);
        assert!(crossings(60, 70).is_empty());
        assert!(crossings(80, 99).is_empty());
        assert_eq!(CrossDirection::Up.as_str(), "up");
        assert_eq!(CrossDirection::Down.as_str(), "down");
    }
}
