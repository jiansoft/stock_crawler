//! 被追蹤股票的備援採集任務。
//!
//! 全市場快取（HiStock、Yahoo 類股）輪到某檔股票前可能已經過了數十秒；這裡只針對 `Trace`
//! 資料表內的股票，每 [`BACKUP_SNAPSHOT_REFRESH_INTERVAL`] 從備援站點補抓一次價格。

use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;
use futures::future;
use rust_decimal::Decimal;
use tokio::task;

use super::{
    BACKUP_ACTIVE_TASKS, BACKUP_LAST_GENERATION, IS_BACKUP_CACHING, publish_price_update,
    wait_for_interval_or_stop,
};
use crate::{
    app::event::trace::stock_price, core::declare, core::util::atomic::decrement_atomic_usize,
    infra::cache::SHARE,
};

const BACKUP_SNAPSHOT_REFRESH_INTERVAL: Duration = Duration::from_secs(15);

/// 啟動被追蹤股票的備援採集背景任務。
///
/// 此任務只採集 `Trace` 資料表中實際被追蹤的股票，並呼叫
/// [`crawler::fetch_stock_price_from_backup_sites`] 取得最新成交價。
/// 採集結果會以「單筆價格更新」方式寫回 `stock_snapshots`，
/// 若價格真的有異動，還會額外發佈價格更新事件，交由 trace evaluator 判斷是否通知。
pub(super) fn start_traced_stock_backup_caching_task() {
    if IS_BACKUP_CACHING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let generation = BACKUP_LAST_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;

    task::spawn(async move {
        let active_tasks = BACKUP_ACTIVE_TASKS.fetch_add(1, Ordering::SeqCst) + 1;
        tracing::info!(
            "追蹤股票備援採集任務啟動 generation={} active_tasks={}",
            generation,
            active_tasks
        );

        while IS_BACKUP_CACHING.load(Ordering::SeqCst) {
            if !declare::StockExchange::TWSE.is_open() {
                break;
            }

            if let Err(why) = refresh_traced_stock_snapshot_cache().await {
                tracing::error!("Failed to refresh traced stock snapshot cache: {:?}", why);
            }

            if !IS_BACKUP_CACHING.load(Ordering::SeqCst) {
                break;
            }

            wait_for_interval_or_stop(BACKUP_SNAPSHOT_REFRESH_INTERVAL).await;
        }

        IS_BACKUP_CACHING.store(false, Ordering::SeqCst);
        let active_tasks = decrement_atomic_usize(&BACKUP_ACTIVE_TASKS);
        tracing::info!(
            "追蹤股票備援採集任務已停止 generation={} active_tasks={}",
            generation,
            active_tasks
        );
    });
}

/// 停止被追蹤股票的備援採集背景任務。
///
/// 此方法只會要求背景迴圈停止，不會直接清空共用的即時報價快取；
/// 快取清理仍交由 crawler 層的背景任務停止流程處理。
pub(super) fn stop_traced_stock_backup_caching_task() {
    IS_BACKUP_CACHING.store(false, Ordering::SeqCst);
}

/// 重新整理「被追蹤股票」的備援即時報價快取。
///
/// 流程如下：
/// 1. 從追蹤條件快取取得目前被追蹤的股票代號。
/// 2. 透過 crawler 的備援站點抓取價格，避免依賴全市場快取是否已輪到該股票。
/// 3. 僅在價格實際異動時，以單筆價格更新方式寫回共用快取並發佈價格事件。
async fn refresh_traced_stock_snapshot_cache() -> Result<()> {
    let symbols = stock_price::get_tracked_symbols();
    if symbols.is_empty() {
        return Ok(());
    }

    let _ = future::join_all(
        symbols
            .into_iter()
            .map(|symbol| async move { refresh_single_traced_stock_snapshot(symbol).await }),
    )
    .await;

    //let updated = results.into_iter().filter(|is_updated| *is_updated).count();
    // tracing::debug!("追蹤股票備援快取已更新 {} 檔", updated);

    Ok(())
}

/// 重新整理單一被追蹤股票的備援即時價格。
async fn refresh_single_traced_stock_snapshot(symbol: String) -> bool {
    match crate::infra::crawler::fetch_stock_price_from_backup_sites_with_source(&symbol).await {
        Ok(result) if result.price != Decimal::ZERO => {
            let price = result.price;
            let source_site = result.site_name.to_string();
            let previous_snapshot = SHARE.get_stock_snapshot(&symbol);
            let price_changed = previous_snapshot
                .as_ref()
                .is_none_or(|snapshot| snapshot.price != price);
            let last_close = previous_snapshot
                .as_ref()
                .map(|s| s.last_close)
                .unwrap_or(Decimal::ZERO);
            if !SHARE.is_valid_price(&symbol, price, last_close) {
                tracing::warn!(
                    "過濾異常價格！股票: {}, 採集價格: {}, 昨收價: {}, 站點: {}",
                    symbol,
                    price,
                    last_close,
                    source_site
                );
                return false;
            }
            let source_changed = previous_snapshot
                .as_ref()
                .is_none_or(|snapshot| snapshot.source_site != source_site);

            if !price_changed && !source_changed {
                return false;
            }

            // 備援採集只負責把價格補進共享快取，
            // 後續警報判斷統一由價格事件 consumer 再從快取讀值。
            SHARE.set_stock_snapshot_price_with_source(symbol.clone(), price, source_site);

            if !price_changed {
                return false;
            }

            publish_price_update(symbol, price);
            true
        }
        Ok(_) => {
            // 備援採集逐檔輪詢時的高頻雜訊，降為 trace 避免日誌暴增。
            tracing::trace!("Stock {} backup price is zero, skipping", symbol);
            false
        }
        Err(why) => {
            // 備援站點全數失敗最常見的原因是冷門股開盤後尚未成交（各站回 `-`、null 或 0），
            // 屬預期狀況；主要報價仍由 HiStock／Yahoo 類股快取提供，因此只記 warn。
            // 2026-09-24 的 55 筆此類 error 全集中在 09:00～10:34，成交後即自行消失。
            tracing::warn!("Failed to fetch backup price for {}: {:#}", symbol, why);
            false
        }
    }
}
