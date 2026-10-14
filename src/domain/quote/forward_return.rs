//! # 未來報酬
//!
//! 技術訊號出現後的表現，以「基準日收盤 → 之後第 N 個交易日收盤」的還原報酬衡量
//! （除權息、減資、分割依參考價還原，等同含息報酬）。交易日 T 收盤時回填 T 往前第 N 個
//! 交易日的 N 日報酬，資料表 `daily_forward_return` 每個基準日一列、每個天期一欄。
//!
//! 反過來看，基準日 T−N 的 N 日報酬就是 T 的近 N 日報酬，相對強度排行直接由此計算。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

/// 回填的天期（交易日數）。120、240 日供相對強度排行使用。
pub const FORWARD_HORIZONS: [usize; 5] = [5, 20, 60, 120, 240];

/// 未來報酬的存取。
#[async_trait]
pub trait ForwardReturnRepository: Send + Sync {
    /// `until`（含）之前最近 `count` 個交易日，由新到舊。
    async fn fetch_trading_dates(&self, until: NaiveDate, count: usize) -> Result<Vec<NaiveDate>>;

    /// 寫入 `base` 當天有成交的上市櫃股票到 `end` 收盤的 `horizon` 日還原報酬，回傳寫入列數；
    /// 其他天期的欄位保留原值。
    async fn upsert_forward_returns(
        &self,
        base: NaiveDate,
        end: NaiveDate,
        horizon: usize,
    ) -> Result<u64>;
}

/// 收盤後衍生資料的筆數，供收盤完整性檢查判斷技術訊號與未來報酬是否寫入。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DerivedCounts {
    /// 資料日的技術訊號筆數。
    pub signals: i64,
    /// 基準日（資料日往前第 5 個交易日）已寫入 5 日報酬的筆數。
    pub forward_returns: i64,
    /// 資料日的每日相對強度筆數。
    pub relative_strength: i64,
}

/// 收盤後衍生資料的筆數查詢。
#[async_trait]
pub trait DerivedDataQuery: Send + Sync {
    /// `date` 的技術訊號與每日相對強度筆數，以及基準日 `base` 已寫入 5 日報酬的筆數。
    async fn count_derived(&self, date: NaiveDate, base: NaiveDate) -> Result<DerivedCounts>;
}

/// `end` 往前第 `horizon` 個交易日（`trading_dates` 由新到舊、第一個必須是 `end`）。
///
/// `end` 不是交易日或交易日不足時回 `None`。
pub fn base_date(end: NaiveDate, horizon: usize, trading_dates: &[NaiveDate]) -> Option<NaiveDate> {
    if trading_dates.first() != Some(&end) {
        return None;
    }
    trading_dates.get(horizon).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).expect("有效日期")
    }

    /// 第 N 個交易日：由新到舊的第 N 個（跳過假日）；結束日不是交易日或日數不足為 None。
    #[test]
    fn base_date_counts_trading_days_back() {
        let dates = [day(13), day(12), day(8), day(7), day(6), day(5), day(2)];
        assert_eq!(base_date(day(13), 1, &dates), Some(day(12)));
        assert_eq!(base_date(day(13), 5, &dates), Some(day(5)));
        assert_eq!(base_date(day(13), 7, &dates), None);
        assert_eq!(base_date(day(11), 1, &dates), None);
        assert_eq!(base_date(day(13), 1, &[]), None);
    }
}
