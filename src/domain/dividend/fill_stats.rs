//! 全市場填息基準：依現金殖利率分組的填息率與填息天數。
//!
//! 2026-10 研究（T13）顯示現金殖利率（要回補的缺口大小）最能區分填息快慢，
//! 沒有自身歷史的股票可拿同組的中位數當參考。純計算、不做 I/O。
//!
//! 只統計除權息後已滿 [`MATURE_TRADING_DAYS`](crate::domain::dividend::fill_stats::MATURE_TRADING_DAYS) 個交易日的事件，各期間填息率與中位數用同一批母體，
//! 才不會被剛除息、還沒機會填息的事件拉低。

use anyhow::Result;
use async_trait::async_trait;
use chrono::{Days, NaiveDate};
use rust_decimal::Decimal;

/// 統計填息需要的觀察期（交易日）；未滿的事件不列入填息率與中位數。
pub const MATURE_TRADING_DAYS: u32 = 120;

/// 現金殖利率分組（現金股利 ÷ 除權息前一日收盤）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum YieldBucket {
    /// 未滿 3%。
    Below3,
    /// 3%（含）～5%。
    From3To5,
    /// 5%（含）～7%。
    From5To7,
    /// 7% 以上。
    AtLeast7,
}

impl YieldBucket {
    /// 由低到高的全部分組。
    pub const ALL: [Self; 4] = [Self::Below3, Self::From3To5, Self::From5To7, Self::AtLeast7];

    /// 依現金股利與除權息前一日收盤分組；沒有現金股利或收盤不大於 0 時為 `None`。
    #[must_use]
    pub fn of(cash_dividend: Decimal, pre_ex_close: Decimal) -> Option<Self> {
        if cash_dividend <= Decimal::ZERO || pre_ex_close <= Decimal::ZERO {
            return None;
        }
        let percent = cash_dividend * Decimal::ONE_HUNDRED / pre_ex_close;
        Some(if percent < Decimal::from(3) {
            Self::Below3
        } else if percent < Decimal::from(5) {
            Self::From3To5
        } else if percent < Decimal::from(7) {
            Self::From5To7
        } else {
            Self::AtLeast7
        })
    }

    /// 對外的代碼（API 與快取鍵用）。
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Below3 => "below_3",
            Self::From3To5 => "from_3_to_5",
            Self::From5To7 => "from_5_to_7",
            Self::AtLeast7 => "at_least_7",
        }
    }

    /// 給人看的區間文字。
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Below3 => "<3%",
            Self::From3To5 => "3～5%",
            Self::From5To7 => "5～7%",
            Self::AtLeast7 => "≥7%",
        }
    }
}

/// 一次除權息的填息樣本。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FillSample {
    pub bucket: YieldBucket,
    /// 除權息日起第幾個交易日填息（觀察期內）；未填息為 `None`。
    pub trading_days_to_fill: Option<u32>,
    /// 除權息日起已觀察的交易日數（最多 [`MATURE_TRADING_DAYS`]）。
    pub trading_days_observed: u32,
}

/// 單一殖利率分組的填息統計。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BucketStats {
    pub bucket: YieldBucket,
    /// 期間內的除權息次數（含觀察期未滿者）。
    pub events: usize,
    /// 觀察期已滿、列入統計的次數。
    pub mature_events: usize,
    /// 20／60／120 個交易日內填息的比例（%，取到小數第 1 位）；沒有可統計的事件為 `None`。
    pub fill_rate_20: Option<f64>,
    pub fill_rate_60: Option<f64>,
    pub fill_rate_120: Option<f64>,
    /// 觀察期內已填息者的填息交易日數中位數。
    pub median_trading_days: Option<f64>,
}

/// 依殖利率分組統計填息率與中位數天數；四組都會回傳（沒有樣本的組次數為 0）。
#[must_use]
pub fn bucket_stats(samples: &[FillSample]) -> Vec<BucketStats> {
    YieldBucket::ALL
        .into_iter()
        .map(|bucket| stats_of(bucket, samples))
        .collect()
}

fn stats_of(bucket: YieldBucket, samples: &[FillSample]) -> BucketStats {
    let in_bucket: Vec<&FillSample> = samples.iter().filter(|s| s.bucket == bucket).collect();
    let mature: Vec<&FillSample> = in_bucket
        .iter()
        .copied()
        .filter(|s| s.trading_days_observed >= MATURE_TRADING_DAYS)
        .collect();
    let rate = |within: u32| {
        if mature.is_empty() {
            return None;
        }
        let filled = mature
            .iter()
            .filter(|s| s.trading_days_to_fill.is_some_and(|days| days <= within))
            .count();
        Some((filled as f64 * 1000.0 / mature.len() as f64).round() / 10.0)
    };
    let mut days: Vec<u32> = mature
        .iter()
        .filter_map(|s| s.trading_days_to_fill)
        .collect();
    days.sort_unstable();
    let median_trading_days = match days.len() {
        0 => None,
        len if len % 2 == 1 => Some(f64::from(days[len / 2])),
        len => Some((f64::from(days[len / 2 - 1]) + f64::from(days[len / 2])) / 2.0),
    };
    BucketStats {
        bucket,
        events: in_bucket.len(),
        mature_events: mature.len(),
        fill_rate_20: rate(20),
        fill_rate_60: rate(60),
        fill_rate_120: rate(MATURE_TRADING_DAYS),
        median_trading_days,
    }
}

/// 統計期間 `(起日（不含）, 迄日（含）)`：`today` 往前 `years` 年（每年 365 天，多 1 天涵蓋閏年）。
#[must_use]
pub fn stats_window(today: NaiveDate, years: u8) -> (NaiveDate, NaiveDate) {
    let days = u64::from(years) * 365 + 1;
    (today - Days::new(days), today)
}

/// 全市場填息樣本的資料來源。
#[async_trait]
pub trait MarketFillQuery: Send + Sync {
    /// 除權息日在 `since`（不含）～`until`（含）之間、上市櫃普通股每次有現金股利的除權息樣本；
    /// 填息目標價依配股調整，與 [`super::fill::fill_target`] 相同。
    async fn fetch_market_fill_samples(
        &self,
        since: NaiveDate,
        until: NaiveDate,
    ) -> Result<Vec<FillSample>>;
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn sample(bucket: YieldBucket, days: Option<u32>, observed: u32) -> FillSample {
        FillSample {
            bucket,
            trading_days_to_fill: days,
            trading_days_observed: observed,
        }
    }

    /// 分組邊界：3%、5%、7% 歸到較高的一組；沒有現金股利或收盤為 0 時不分組。
    #[test]
    fn yield_bucket_boundaries() {
        assert_eq!(
            YieldBucket::of(dec!(2.99), dec!(100)),
            Some(YieldBucket::Below3)
        );
        assert_eq!(
            YieldBucket::of(dec!(3), dec!(100)),
            Some(YieldBucket::From3To5)
        );
        assert_eq!(
            YieldBucket::of(dec!(5), dec!(100)),
            Some(YieldBucket::From5To7)
        );
        assert_eq!(
            YieldBucket::of(dec!(7), dec!(100)),
            Some(YieldBucket::AtLeast7)
        );
        assert_eq!(YieldBucket::of(Decimal::ZERO, dec!(100)), None);
        assert_eq!(YieldBucket::of(dec!(1), Decimal::ZERO), None);
    }

    /// 填息率與中位數只看觀察期已滿的事件；未滿的只算進 events。
    #[test]
    fn bucket_stats_uses_mature_events_only() {
        let samples = [
            sample(YieldBucket::Below3, Some(1), 120),
            sample(YieldBucket::Below3, Some(30), 120),
            sample(YieldBucket::Below3, Some(90), 120),
            sample(YieldBucket::Below3, None, 120),
            sample(YieldBucket::Below3, None, 15),
            sample(YieldBucket::Below3, Some(2), 5),
            sample(YieldBucket::AtLeast7, None, 120),
        ];
        let stats = bucket_stats(&samples);
        assert_eq!(
            stats.iter().map(|s| s.bucket).collect::<Vec<_>>(),
            YieldBucket::ALL
        );
        assert_eq!(
            stats[0],
            BucketStats {
                bucket: YieldBucket::Below3,
                events: 6,
                mature_events: 4,
                fill_rate_20: Some(25.0),
                fill_rate_60: Some(50.0),
                fill_rate_120: Some(75.0),
                median_trading_days: Some(30.0),
            }
        );
        let high = stats[3];
        assert_eq!((high.mature_events, high.fill_rate_120), (1, Some(0.0)));
        assert_eq!(high.median_trading_days, None, "沒有已填息者");
    }

    /// 5 年期間與持股填息的近 5 年歷史一致（5 × 365 + 1 天）。
    #[test]
    fn stats_window_spans_whole_years() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 31).expect("日期應合法");
        assert_eq!(
            stats_window(today, 5),
            (
                NaiveDate::from_ymd_opt(2021, 10, 31).expect("日期應合法"),
                today
            )
        );
    }

    /// 沒有樣本的組次數為 0、比例與中位數為 None；偶數筆中位數取中間兩筆平均。
    #[test]
    fn bucket_stats_handles_empty_and_even_counts() {
        let stats = bucket_stats(&[
            sample(YieldBucket::From3To5, Some(4), 120),
            sample(YieldBucket::From3To5, Some(10), 120),
        ]);
        assert_eq!(stats[1].median_trading_days, Some(7.0));
        let empty = stats[2];
        assert_eq!(
            (empty.events, empty.fill_rate_20, empty.median_trading_days),
            (0, None, None)
        );
        assert_eq!(YieldBucket::AtLeast7.code(), "at_least_7");
        assert_eq!(YieldBucket::From5To7.label(), "5～7%");
    }
}
