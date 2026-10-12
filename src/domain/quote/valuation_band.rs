//! # 本益比／股價淨值比歷史區間
//!
//! 把一段期間內每日的本益比（或股價淨值比）當成樣本，回答「目前的估值在自己歷史裡
//! 算高還是低」：目前值的百分位、最低／P25／中位數／P75／最高，以及估值回到各分位時
//! 對應的股價。
//!
//! - 本益比為 0 或負數代表該日近四季虧損或交易所未提供，不是「很便宜」，一律排除，
//!   另計排除天數讓呼叫端知道樣本少了多少。股價淨值比同理。
//! - 分位數採線性內插（與 PostgreSQL `percentile_cont` 相同）。
//! - 百分位＝樣本中「小於或等於目前值」的比例（0–100）。
//! - 對應股價＝最新收盤價 × 分位值 ÷ 目前值，假設獲利（淨值）不變、只有估值倍數改變。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::{Decimal, prelude::ToPrimitive};

/// 單日的收盤價與估值倍數。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyRatio {
    /// 交易日。
    pub date: NaiveDate,
    /// 收盤價。
    pub close: Decimal,
    /// 本益比；虧損或未提供時為 0。
    pub price_earning_ratio: Decimal,
    /// 股價淨值比；未提供時為 0。
    pub price_to_book_ratio: Decimal,
}

/// 估值區間的資料來源。
#[async_trait]
pub trait ValuationHistoryRepository: Send + Sync {
    /// 以最新收盤日往回 `years` 年的每日倍數，依日期由新到舊；沒有成交（收盤價 0）的占位列不算。
    async fn fetch_daily_ratios(&self, stock_symbol: &str, years: u8) -> Result<Vec<DailyRatio>>;
}

/// 單一估值倍數（本益比或股價淨值比）的歷史區間統計。
#[derive(Debug, Clone, PartialEq)]
pub struct RatioBand {
    /// 最新一日的倍數；該日為 0 或負數（虧損、未提供）時為 `None`。
    pub current: Option<f64>,
    /// 目前值在樣本中的百分位（0–100，四捨五入到小數一位）；沒有目前值時為 `None`。
    pub percentile: Option<f64>,
    /// 最低值。
    pub min: f64,
    /// 第 25 百分位。
    pub p25: f64,
    /// 中位數。
    pub median: f64,
    /// 第 75 百分位。
    pub p75: f64,
    /// 最高值。
    pub max: f64,
    /// 有效樣本天數（倍數 > 0）。
    pub samples: usize,
    /// 因倍數 ≤ 0 被排除的天數。
    pub excluded: usize,
    /// 倍數回到 P25／中位數／P75 時對應的股價；沒有目前值或收盤價時為 `None`。
    pub implied_prices: Option<ImpliedPrices>,
}

/// 估值倍數回到各分位時對應的股價（元，四捨五入到小數兩位）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImpliedPrices {
    /// 回到 P25。
    pub p25: f64,
    /// 回到中位數。
    pub median: f64,
    /// 回到 P75。
    pub p75: f64,
}

/// 計算單一倍數的歷史區間。
///
/// `history` 是期間內每日的倍數（順序不拘），`current` 是最新一日的倍數，
/// `closing_price` 是最新一日的收盤價。有效樣本為 0 時回傳 `None`。
pub fn summarize(
    history: &[Decimal],
    current: Option<Decimal>,
    closing_price: Option<Decimal>,
) -> Option<RatioBand> {
    let mut values: Vec<f64> = history
        .iter()
        .filter(|value| value.is_sign_positive() && !value.is_zero())
        .filter_map(ToPrimitive::to_f64)
        .collect();
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let excluded = history.len() - values.len();

    let current = current
        .filter(|value| value.is_sign_positive() && !value.is_zero())
        .and_then(|value| value.to_f64());
    let percentile = current.map(|value| {
        let at_or_below = values.partition_point(|sample| *sample <= value);
        round_to(at_or_below as f64 / values.len() as f64 * 100.0, 1)
    });

    let p25 = quantile(&values, 0.25);
    let median = quantile(&values, 0.5);
    let p75 = quantile(&values, 0.75);
    let implied_prices = current
        .zip(closing_price.and_then(|price| price.to_f64()))
        .filter(|(_, price)| *price > 0.0)
        .map(|(ratio, price)| ImpliedPrices {
            p25: round_to(price * p25 / ratio, 2),
            median: round_to(price * median / ratio, 2),
            p75: round_to(price * p75 / ratio, 2),
        });

    Some(RatioBand {
        current,
        percentile,
        min: values[0],
        p25: round_to(p25, 4),
        median: round_to(median, 4),
        p75: round_to(p75, 4),
        max: values[values.len() - 1],
        samples: values.len(),
        excluded,
        implied_prices,
    })
}

/// 已排序樣本的分位數，相鄰兩點線性內插（PostgreSQL `percentile_cont`）。
fn quantile(sorted: &[f64], fraction: f64) -> f64 {
    let position = fraction * (sorted.len() - 1) as f64;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    sorted[lower] + (sorted[upper] - sorted[lower]) * (position - lower as f64)
}

/// 四捨五入到 `digits` 位小數。
fn round_to(value: f64, digits: i32) -> f64 {
    let factor = 10f64.powi(digits);
    (value * factor).round() / factor
}

#[cfg(test)]
mod tests {
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
}
