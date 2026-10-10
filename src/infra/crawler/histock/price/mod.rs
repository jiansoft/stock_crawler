//! # HiStock 即時報價採集 (全市場快取版)
//!
//! 此模組負責透過 HiStock 的排行榜頁面取得台股即時報價資料。
//!
//! ## 核心設計：全市場智慧快取
//! 1. **全欄位快取**：包含代號、名稱、成交、漲跌、幅、開盤、最高、最低、昨收、成交量。
//! 2. **外部驅動啟停**：
//!    - 依賴外部事件 (如 `src/event/trace/stock_price.rs`) 在開盤期間啟動定時任務。
//!    - 依賴收盤事件停止任務。停止後會清空快取以節省記憶體並確保下次啟動時資料新鮮。
//! 3. **消費者共用**：背景任務更新後的資料會寫入 [`SHARE`](infra::cache::SHARE)
//!    的 `stock_snapshots`，供追蹤任務與 `StockInfo` 介面共用。
//! 4. **嚴格解析**：不容忍損壞或格式錯誤的報價，解析失敗會傳回錯誤而非默默變 0。
//!
//! ## 檔案分工
//! - `parse`：排行頁 HTML 解析（純函式）。
//! - `screen`：剔除前後矛盾的列，錯亂太多時整批捨棄。
//! - `task`：開盤期間的背景快取任務與診斷資訊。

use std::collections::HashMap;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use rust_decimal::Decimal;
use tokio::sync::Mutex;

use crate::{
    app::event::trace::price_tasks as trace_price_tasks,
    core::declare,
    core::util,
    infra::cache::{RealtimeSnapshot, SHARE},
    infra::crawler::{
        StockInfo,
        histock::{HOST, HiStock},
    },
};

mod parse;
mod screen;
mod task;

pub(crate) use task::{diagnostics_snapshot, runtime_diagnostics_snapshot};
pub use task::{start_caching_task, stop_caching_task};

/// 用於解決 Single-flight (重複抓取) 的互斥鎖
static FETCH_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// HiStock 抓取結果的內部資料結構。
///
/// 用於封裝解析後的即時報價快照以及傳輸量、資料列數等診斷資訊。
#[derive(Debug)]
struct HiStockFetchResult {
    snapshots: HashMap<String, RealtimeSnapshot>,
    body_bytes: usize,
    row_count: usize,
}

/// 從 HiStock 排行榜抓取全市場即時報價資料。
///
/// 只負責 HTTP 抓取，解析工作交給 [`parse::parse_rank_html`]（fetch/parse 分離，
/// 解析邏輯才能被 fixture 單元測試覆蓋）。
async fn fetch_all_from_rank() -> Result<HiStockFetchResult> {
    let url = format!("https://{host}/stock/rank.aspx?p=all", host = HOST);
    let body = util::http::get(&url, None).await?;
    screen_rank_page(&body)
}

/// 解析排行頁並剔除前後矛盾的列；除權息參考價取自全域快取。
fn screen_rank_page(body: &str) -> Result<HiStockFetchResult> {
    let mut result = parse::parse_rank_html(body)?;
    screen::screen_inconsistent_rows(&mut result, |symbol| {
        SHARE.get_ex_rights_reference_price(symbol)
    })?;
    Ok(result)
}

/// 取得指定股票的即時快照。
///
/// 讀取策略如下：
/// 1. 先直接查詢全市場快取。
/// 2. 若快取未命中，使用 single-flight 鎖避免多個呼叫端同時觸發全量重抓。
/// 3. 抓取成功後，以新資料覆蓋全市場快取，再回傳目標股票快照。
async fn get_snapshot(stock_symbol: &str) -> Result<RealtimeSnapshot> {
    // 第一階段：嘗試取得快取
    if let Some(s) = SHARE.get_stock_snapshot(stock_symbol) {
        return Ok(s);
    }

    // 第二階段：快取失效，使用互斥鎖防止重複抓取 (Single-flight)
    let _lock = FETCH_LOCK.lock().await;

    // 拿到鎖後再檢查一次快取，可能剛才有人抓過了
    if let Some(s) = SHARE.get_stock_snapshot(stock_symbol) {
        return Ok(s);
    }

    tracing::info!("HiStock 快取失效 ({})，觸發全量抓取", stock_symbol);
    let fetch_result = fetch_all_from_rank().await?;
    store_and_pick(stock_symbol, fetch_result)
}

/// 以全量重抓的結果覆蓋快取，並取出指定股票的快照。
///
/// 目標股票不在排行頁時回傳錯誤，且不更新快取（與拆出前相同）。
fn store_and_pick(
    stock_symbol: &str,
    fetch_result: HiStockFetchResult,
) -> Result<RealtimeSnapshot> {
    let snapshot = fetch_result
        .snapshots
        .get(stock_symbol)
        .cloned()
        .ok_or_else(|| {
            anyhow!(
                "Stock symbol {} not found in HiStock rank page after refresh",
                stock_symbol
            )
        })?;

    // 更新全量快取
    let price_updates = task::collect_changed_price_updates(&fetch_result.snapshots);
    SHARE.set_stock_snapshots(fetch_result.snapshots);
    trace_price_tasks::publish_price_updates(price_updates);

    Ok(snapshot)
}

#[async_trait]
impl StockInfo for HiStock {
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal> {
        let snapshot = get_snapshot(stock_symbol).await?;
        Ok(snapshot.price)
    }

    async fn get_stock_quotes(stock_symbol: &str) -> Result<declare::StockQuotes> {
        let snapshot = get_snapshot(stock_symbol).await?;

        snapshot.try_into_stock_quotes()
    }
}

#[cfg(test)]
mod tests;
