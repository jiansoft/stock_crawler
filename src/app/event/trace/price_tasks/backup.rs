//! 被追蹤股票的備援採集任務。
//!
//! 全市場快取（HiStock、Yahoo 類股）輪到某檔股票前可能已經過了數十秒；這裡只針對 `Trace`
//! 資料表內的股票，每 [`BACKUP_SNAPSHOT_REFRESH_INTERVAL`] 從備援站點補抓一次價格。

use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;
use chrono::Local;
use futures::future;
use once_cell::sync::Lazy;
use rust_decimal::Decimal;
use tokio::task;

use super::{
    BACKUP_ACTIVE_TASKS, BACKUP_LAST_GENERATION, IS_BACKUP_CACHING, publish_price_update,
    wait_for_interval_or_stop,
};
use crate::{
    app::event::trace::stock_price,
    core::declare,
    core::util::{atomic::decrement_atomic_usize, daily_seen::DailySeen},
    infra::cache::{SHARE, report_abnormal_price},
    infra::crawler::FetchedStockPrice,
};

const BACKUP_SNAPSHOT_REFRESH_INTERVAL: Duration = Duration::from_secs(15);

/// 今天已用 warn 記過備援抓價失敗的股票。
static BACKUP_FAILURE_WARNED: Lazy<Mutex<DailySeen>> =
    Lazy::new(|| Mutex::new(DailySeen::default()));

/// 這檔股票今天是否第一次備援抓價失敗；鎖中毒時一律視為第一次。
fn first_backup_failure_today(symbol: &str) -> bool {
    BACKUP_FAILURE_WARNED
        .lock()
        .map(|mut seen| seen.first_today(symbol, Local::now().date_naive()))
        .unwrap_or(true)
}

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
    let fetched =
        crate::infra::crawler::fetch_stock_price_from_backup_sites_with_source(&symbol).await;
    apply_backup_price(symbol, fetched)
}

/// 把備援站點的抓價結果寫回共享快取。
///
/// 只有價格實際異動時才發佈價格事件並回傳 `true`；只換了來源站點時更新快取但回傳 `false`。
/// 與網路抓取分開，才能在不連外部網站的情況下驗證過濾與更新規則。
fn apply_backup_price(symbol: String, fetched: Result<FetchedStockPrice>) -> bool {
    match fetched {
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
                report_abnormal_price(&symbol, price, last_close, &source_site);
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
            // 同一檔每天只記第一次 warn，之後的重複失敗記 debug。
            if first_backup_failure_today(&symbol) {
                tracing::warn!("Failed to fetch backup price for {}: {:#}", symbol, why);
            } else {
                tracing::debug!("Failed to fetch backup price for {}: {:#}", symbol, why);
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;
    use crate::infra::cache::RealtimeSnapshot;

    /// 不存在於股票主檔的測試代號，避免與真實報價或其他測試互相干擾。
    const SYMBOL_NEW: &str = "79961";
    const SYMBOL_SAME: &str = "79962";
    const SYMBOL_ABNORMAL: &str = "79963";
    const SYMBOL_ZERO: &str = "79964";

    fn fetched(price: Decimal, site_name: &'static str) -> Result<FetchedStockPrice> {
        Ok(FetchedStockPrice { price, site_name })
    }

    fn put_snapshot(symbol: &str, price: Decimal, last_close: Decimal, source_site: &str) {
        let mut snapshot = RealtimeSnapshot::new(symbol.to_string(), price);
        snapshot.last_close = last_close;
        snapshot.source_site = source_site.to_string();
        SHARE
            .stock_snapshots
            .write()
            .expect("snapshot lock")
            .insert(symbol.to_string(), snapshot);
    }

    fn remove_snapshot(symbol: &str) {
        SHARE
            .stock_snapshots
            .write()
            .expect("snapshot lock")
            .remove(symbol);
    }

    /// 快取沒有這檔時寫入新價格與來源，並視為價格異動。
    #[test]
    fn new_price_is_cached_and_reported_as_changed() {
        remove_snapshot(SYMBOL_NEW);

        assert!(apply_backup_price(
            SYMBOL_NEW.to_string(),
            fetched(dec!(101.5), "CnYes")
        ));
        let snapshot = SHARE.get_stock_snapshot(SYMBOL_NEW).expect("應已寫入快取");
        assert_eq!(snapshot.price, dec!(101.5));
        assert_eq!(snapshot.source_site, "CnYes");

        remove_snapshot(SYMBOL_NEW);
    }

    /// 價格與來源都沒變不更新；只換來源時更新來源但不算價格異動。
    #[test]
    fn unchanged_price_is_not_reported() {
        put_snapshot(SYMBOL_SAME, dec!(50), dec!(50), "Fugle");

        assert!(!apply_backup_price(
            SYMBOL_SAME.to_string(),
            fetched(dec!(50), "Fugle")
        ));
        assert!(!apply_backup_price(
            SYMBOL_SAME.to_string(),
            fetched(dec!(50), "PcHome")
        ));
        let snapshot = SHARE.get_stock_snapshot(SYMBOL_SAME).expect("快取應存在");
        assert_eq!(snapshot.source_site, "PcHome");
        assert_eq!(snapshot.price, dec!(50));

        remove_snapshot(SYMBOL_SAME);
    }

    /// 超出漲跌幅的價格（HiStock 偶發錯價那類）必須被過濾，快取維持原值。
    #[test]
    fn abnormal_price_is_filtered() {
        put_snapshot(SYMBOL_ABNORMAL, dec!(54.7), dec!(54.7), "Yahoo");

        assert!(!apply_backup_price(
            SYMBOL_ABNORMAL.to_string(),
            fetched(dec!(141.5), "CMoney")
        ));
        let snapshot = SHARE
            .get_stock_snapshot(SYMBOL_ABNORMAL)
            .expect("快取應存在");
        assert_eq!(snapshot.price, dec!(54.7));
        assert_eq!(snapshot.source_site, "Yahoo");

        remove_snapshot(SYMBOL_ABNORMAL);
    }

    /// 0 元（尚未成交）與抓取失敗都不寫入快取。
    #[test]
    fn zero_price_and_fetch_errors_are_ignored() {
        remove_snapshot(SYMBOL_ZERO);

        assert!(!apply_backup_price(
            SYMBOL_ZERO.to_string(),
            fetched(Decimal::ZERO, "CnYes")
        ));
        assert!(!apply_backup_price(
            SYMBOL_ZERO.to_string(),
            Err(anyhow::anyhow!("all backup sites failed"))
        ));
        assert!(SHARE.get_stock_snapshot(SYMBOL_ZERO).is_none());
    }

    /// 沒有被追蹤的股票時不發任何請求，直接成功。
    #[tokio::test]
    async fn refresh_without_traced_symbols_is_a_noop() {
        stock_price::clear_trace_targets_cache();
        refresh_traced_stock_snapshot_cache()
            .await
            .expect("空清單應直接成功");
    }

    /// 背景任務可以啟動與停止；非交易時段會自行結束，交易時段則由停止旗標結束。
    #[tokio::test(start_paused = true)]
    async fn backup_task_starts_and_stops() {
        stock_price::clear_trace_targets_cache();
        start_traced_stock_backup_caching_task();

        stop_traced_stock_backup_caching_task();
        for _ in 0..100 {
            super::super::TRACE_TASK_STOP_NOTIFY.notify_waiters();
            tokio::time::sleep(Duration::from_millis(50)).await;
            if BACKUP_ACTIVE_TASKS.load(Ordering::SeqCst) == 0
                && !IS_BACKUP_CACHING.load(Ordering::SeqCst)
            {
                return;
            }
        }
        panic!("備援採集任務未在期限內停止");
    }
}
