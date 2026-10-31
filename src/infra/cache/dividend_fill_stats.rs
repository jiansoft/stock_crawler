//! 全市場填息樣本的短期快取：`/market/dividend-fill-stats`、持股的同組基準與填息通知共用。
//!
//! 樣本只會在收盤後隨新日線與新除權息事件改變，一次查詢約 0.7～3 秒，快取 6 小時；
//! 同鍵未命中時只執行一次查詢，錯誤不快取。

use std::{future::Future, sync::Arc, time::Duration};

use anyhow::Result;
use chrono::NaiveDate;
use moka::future::Cache;
use once_cell::sync::Lazy;

use super::etf_analysis::shared_error;
use crate::domain::dividend::fill_stats::{FillSample, MarketFillQuery, stats_window};

/// 樣本快取存活時間。
const TIME_TO_LIVE: Duration = Duration::from_secs(6 * 3600);

/// 快取鍵：統計期間（起日、迄日）。
type WindowKey = (NaiveDate, NaiveDate);

/// 依統計期間快取的全市場填息樣本。
static SAMPLES: Lazy<Cache<WindowKey, Arc<Vec<FillSample>>>> = Lazy::new(|| {
    Cache::builder()
        .max_capacity(16)
        .time_to_live(TIME_TO_LIVE)
        .build()
});

/// 取得 `since`（不含）～`until`（含）的全市場填息樣本；未命中時以 `load` 查詢並快取。
///
/// # Errors
///
/// `load` 失敗時回傳其錯誤（保留原因鏈），且不寫入快取。
pub async fn market_fill_samples(
    since: NaiveDate,
    until: NaiveDate,
    load: impl Future<Output = Result<Vec<FillSample>>>,
) -> Result<Arc<Vec<FillSample>>> {
    SAMPLES
        .try_get_with((since, until), async { load.await.map(Arc::new) })
        .await
        .map_err(shared_error)
}

/// `today` 往前 `years` 年的全市場填息樣本（[`stats_window`]），經快取。
///
/// # Errors
///
/// 查詢失敗時回傳錯誤。
pub async fn market_fill_samples_for(
    query: &impl MarketFillQuery,
    today: NaiveDate,
    years: u8,
) -> Result<Arc<Vec<FillSample>>> {
    let (since, until) = stats_window(today, years);
    market_fill_samples(since, until, query.fetch_market_fill_samples(since, until)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::dividend::fill_stats::YieldBucket;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(1999, 1, d).expect("日期應合法")
    }

    /// 命中時不再查詢；錯誤不快取，下一次會重新查詢。
    #[tokio::test]
    async fn caches_samples_but_not_errors() {
        let sample = FillSample {
            bucket: YieldBucket::Below3,
            trading_days_to_fill: Some(1),
            trading_days_observed: 120,
        };
        let failed =
            market_fill_samples(day(1), day(2), async { Err(anyhow::anyhow!("db down")) }).await;
        assert_eq!(failed.expect_err("應回傳錯誤").to_string(), "db down");

        let first = market_fill_samples(day(1), day(2), async { Ok(vec![sample]) })
            .await
            .expect("查詢成功");
        let cached = market_fill_samples(day(1), day(2), async {
            panic!("已快取，不應再查詢");
        })
        .await
        .expect("命中快取");
        assert_eq!((first.len(), cached.as_slice()), (1, [sample].as_slice()));
    }
}
