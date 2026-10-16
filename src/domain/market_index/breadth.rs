//! 市場廣度：上市櫃合併站上 20／60／240 日均線的股票比例與創 52 週新高／新低的家數。
//!
//! 資料來自每日收盤後的股價分布統計（`daily_stock_price_stats`，市場 id 0）；
//! Data API 的 `/market/regime` 與每週市場摘要共用這裡的計算，數值不四捨五入，由呈現端決定位數。
//! 2022-02-17 以前的統計母體是全部非 ETF 日線、之後是有估價的股票，家數不連續，比較時看比例。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

/// 一天的市場廣度原始家數。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BreadthDay {
    /// 統計日期。
    pub date: NaiveDate,
    /// 收盤高於 20 日均線的家數。
    pub above_ma20: i32,
    /// 收盤不高於 20 日均線的家數。
    pub below_ma20: i32,
    /// 收盤高於 60 日均線的家數。
    pub above_ma60: i32,
    /// 收盤不高於 60 日均線的家數。
    pub below_ma60: i32,
    /// 收盤高於 240 日均線的家數。
    pub above_ma240: i32,
    /// 收盤不高於 240 日均線的家數。
    pub below_ma240: i32,
    /// 創 52 週新高的家數。
    pub new_high: i32,
    /// 創 52 週新低的家數。
    pub new_low: i32,
}

/// 資料日的市場廣度與近 `window` 個交易日的變化。
#[derive(Debug, Clone, PartialEq)]
pub struct MarketBreadth {
    /// 統計日期（`until` 之前最近一個有統計的交易日）。
    pub date: NaiveDate,
    /// 站上 20 日均線的比例（%）；當天沒有家數為 `None`。
    pub above_ma20_percent: Option<f64>,
    /// 站上 60 日均線的比例（%）。
    pub above_ma60_percent: Option<f64>,
    /// 站上 240 日均線的比例（%）。
    pub above_ma240_percent: Option<f64>,
    /// `window` 個交易日前站上 60 日均線的比例（%）；資料不足為 `None`。
    pub above_ma60_percent_before: Option<f64>,
    /// 近 `window` 個交易日（含資料日）創 52 週新高的家次合計。
    pub new_highs: i64,
    /// 近 `window` 個交易日創 52 週新低的家次合計。
    pub new_lows: i64,
    /// 實際加總的交易日數（資料不足時小於 `window`）。
    pub days: usize,
}

/// 市場廣度統計的查詢介面。
#[async_trait]
pub trait BreadthQuery: Send + Sync {
    /// 上市櫃合併在 `until`（含，`None` 為不限）之前最近 `limit` 個交易日的市場廣度，由新到舊。
    async fn fetch_breadth(
        &self,
        until: Option<NaiveDate>,
        limit: usize,
    ) -> Result<Vec<BreadthDay>>;
}

/// 讀取 `until` 之前最近 `window + 1` 個交易日並彙總；沒有統計回 `None`。
///
/// # Errors
///
/// 查詢失敗時回傳錯誤。
pub async fn load(
    query: &impl BreadthQuery,
    until: Option<NaiveDate>,
    window: usize,
) -> Result<Option<MarketBreadth>> {
    let days = query.fetch_breadth(until, window + 1).await?;
    Ok(summarize(&days, window))
}

/// 由新到舊的市場廣度彙總：資料日的比例、`window` 個交易日前的 60 日均線比例與近 `window`
/// 日的新高／新低家次；沒有資料回 `None`。
pub fn summarize(days: &[BreadthDay], window: usize) -> Option<MarketBreadth> {
    let latest = days.first()?;
    let recent = &days[..window.clamp(1, days.len())];
    Some(MarketBreadth {
        date: latest.date,
        above_ma20_percent: percent(latest.above_ma20, latest.below_ma20),
        above_ma60_percent: percent(latest.above_ma60, latest.below_ma60),
        above_ma240_percent: percent(latest.above_ma240, latest.below_ma240),
        above_ma60_percent_before: days
            .get(window)
            .and_then(|day| percent(day.above_ma60, day.below_ma60)),
        new_highs: recent.iter().map(|day| i64::from(day.new_high)).sum(),
        new_lows: recent.iter().map(|day| i64::from(day.new_low)).sum(),
        days: recent.len(),
    })
}

/// 站上均線的比例（%）；沒有家數為 `None`。
fn percent(above: i32, below: i32) -> Option<f64> {
    let total = above + below;
    (total > 0).then(|| f64::from(above) * 100.0 / f64::from(total))
}

#[cfg(test)]
mod tests {
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
}
