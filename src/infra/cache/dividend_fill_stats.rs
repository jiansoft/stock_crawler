//! 全市場填息樣本的短期快取：`/market/dividend-fill-stats`、持股的同組基準與填息通知共用。
//!
//! 樣本只會在收盤後隨新日線與新除權息事件改變，一次查詢約 0.7～3 秒；同鍵未命中時只執行一次查詢，
//! 錯誤不快取。預設期間（[`MARKET_PEER_YEARS`] 年）由啟動與每 6 小時的排程以
//! [`refresh_market_fill_samples_for`] 先查好覆蓋，請求不必現查：2026-11-05 Rust 重啟後第一次股利預估
//! 遇上 pi 部署中的高負載，現查超過 Go 端 10 秒逾時而回 504。

use std::{future::Future, sync::Arc, time::Duration};

use anyhow::Result;
use chrono::NaiveDate;
use moka::future::Cache;
use once_cell::sync::Lazy;

use super::etf_analysis::shared_error;
#[cfg(doc)]
use crate::domain::dividend::fill_stats::MARKET_PEER_YEARS;
use crate::domain::dividend::fill_stats::{FillSample, MarketFillQuery, stats_window};

/// 樣本快取存活時間：比排程刷新間隔（6 小時）多 1 小時，刷新晚一點或失敗一次時仍讀得到舊值。
const TIME_TO_LIVE: Duration = Duration::from_secs(7 * 3600);

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

/// 重新查詢 `today` 往前 `years` 年的全市場填息樣本並覆蓋快取，回傳樣本數。
///
/// 先查好再覆蓋、不先失效：查詢期間的請求照樣讀舊值，不會因刷新而改為現查。
///
/// # Errors
///
/// 查詢失敗時回傳錯誤，原有的快取保留不動。
pub async fn refresh_market_fill_samples_for(
    query: &impl MarketFillQuery,
    today: NaiveDate,
    years: u8,
) -> Result<usize> {
    let window = stats_window(today, years);
    let samples = query.fetch_market_fill_samples(window.0, window.1).await?;
    let count = samples.len();
    SAMPLES.insert(window, Arc::new(samples)).await;
    Ok(count)
}

#[cfg(test)]
mod tests;
