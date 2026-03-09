//! # Trace 即時價格任務協調模組
//!
//! 此模組負責管理 trace 事件所需的即時報價背景任務，
//! 讓 [`stock_price`](crate::event::trace::stock_price) 可以專注在
//! 價格追蹤與警報邏輯。
//!
//! 目前此模組會協調五種工作：
//! 1. crawler 層的全市場即時報價背景任務（目前由 Yahoo 類股快取驅動）
//! 2. 只針對 `Trace` 資料表內股票的備援採集任務
//! 3. 價格更新事件 consumer，將指定股票的最新價格交給追蹤 evaluator
//! 4. 追蹤條件快取刷新任務，定期同步最新 `trace` 設定
//! 5. 低頻 reconciliation 任務，補償事件遺漏或剛新增追蹤條件的情況

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::time::Duration;

use anyhow::Result;
use futures::future;
use once_cell::sync::Lazy;
use rust_decimal::Decimal;
use tokio::{
    sync::mpsc::{self, UnboundedSender},
    task, time,
};

use super::{stats as trace_stats, stock_price};
use crate::{cache::SHARE, crawler, declare, logging};

/// 價格更新事件。
#[derive(Debug, Clone)]
struct PriceUpdateEvent {
    symbol: String,
    price: Decimal,
}

/// 確保「被追蹤股票備援採集」只有一個背景任務在執行。
static IS_BACKUP_CACHING: AtomicBool = AtomicBool::new(false);
/// 確保「追蹤條件快取刷新」只有一個背景任務在執行。
static IS_TARGET_CACHE_REFRESHING: AtomicBool = AtomicBool::new(false);
/// 確保「低頻 reconciliation」只有一個背景任務在執行。
static IS_RECONCILING: AtomicBool = AtomicBool::new(false);
/// 價格更新事件 sender；存在時代表 consumer 仍可接收事件。
static PRICE_UPDATE_TX: Lazy<RwLock<Option<UnboundedSender<PriceUpdateEvent>>>> =
    Lazy::new(|| RwLock::new(None));
const SNAPSHOT_WARMUP_TIMEOUT: Duration = Duration::from_secs(3);
const SNAPSHOT_WARMUP_POLL_INTERVAL: Duration = Duration::from_millis(100);
const BACKUP_SNAPSHOT_REFRESH_INTERVAL: Duration = Duration::from_secs(15);
const TRACE_TARGET_REFRESH_INTERVAL: Duration = Duration::from_secs(60);
const TRACE_RECONCILIATION_INTERVAL: Duration = Duration::from_secs(60 * 5);

/// 啟動 trace 事件所需的即時報價背景任務。
///
/// 啟動順序如下：
/// 1. 先載入追蹤條件快取，避免第一批價格事件發生時尚無條件可判斷。
/// 2. 啟動價格更新事件 consumer。
/// 3. 啟動追蹤條件快取刷新與低頻 reconciliation。
/// 4. 啟動 crawler 層的全市場即時報價背景任務。
/// 5. 啟動被追蹤股票的備援採集任務。
///
/// # Errors
///
/// 若初次載入追蹤條件快取失敗，將回傳 `Err`，避免追蹤系統在未準備完成時啟動。
pub async fn start_price_tasks() -> Result<()> {
    trace_stats::reset_runtime_stats();

    let traced_symbol_count = stock_price::refresh_trace_targets_cache().await?;
    logging::info_file_async(format!(
        "追蹤條件快取初始化完成，共 {} 檔股票",
        traced_symbol_count
    ));

    start_price_update_consumer_task();
    start_trace_target_refresh_task();
    start_trace_reconciliation_task();
    crawler::price_tasks::start_price_tasks();
    start_traced_stock_backup_caching_task();

    Ok(())
}

/// 等待 trace 所依賴的即時報價快取至少先暖身一次。
///
/// 準備完成的判定條件如下：
/// - 追蹤條件快取已至少成功載入過一次
/// - [`SHARE`](crate::cache::SHARE) 內的 `stock_snapshots` 至少已有一筆資料
///
/// 若在限定時間內仍尚未滿足條件，追蹤流程仍會繼續，
/// 只是開盤初期部分股票可能因快取未命中而暫時略過。
pub async fn wait_for_price_cache_ready() {
    let started_at = std::time::Instant::now();

    while started_at.elapsed() < SNAPSHOT_WARMUP_TIMEOUT {
        let is_ready = stock_price::has_loaded_trace_targets_cache()
            && SHARE
                .stock_snapshots
                .read()
                .map(|cache| !cache.is_empty())
                .unwrap_or(false);

        if is_ready {
            return;
        }

        time::sleep(SNAPSHOT_WARMUP_POLL_INTERVAL).await;
    }

    logging::debug_file_async("股票追蹤快取暖身逾時，先以目前快取內容執行".to_string());
}

/// 停止 trace 事件所需的即時報價背景任務。
///
/// 目前會停止：
/// - 被追蹤股票備援採集任務
/// - 價格更新事件 consumer
/// - 追蹤條件快取刷新任務
/// - 低頻 reconciliation 任務
/// - crawler 層的全市場即時報價背景任務
pub async fn stop_price_tasks() {
    stop_traced_stock_backup_caching_task();
    stop_trace_target_refresh_task();
    stop_trace_reconciliation_task();
    stop_price_update_consumer_task();
    crawler::price_tasks::stop_price_tasks().await;
    trace_stats::flush_runtime_stats();
}

/// 發佈單筆價格更新事件。
///
/// 若 trace 價格事件 consumer 尚未啟動或已停止，事件會直接被忽略。
pub fn publish_price_update(symbol: String, price: Decimal) {
    if price == Decimal::ZERO {
        return;
    }

    if let Ok(tx) = PRICE_UPDATE_TX.read() {
        if let Some(tx) = tx.as_ref() {
            trace_stats::record_published_price_event();
            if tx.send(PriceUpdateEvent { symbol, price }).is_err() {
                trace_stats::record_dropped_price_event();
            }
        } else {
            trace_stats::record_dropped_price_event();
        }
    } else {
        trace_stats::record_dropped_price_event();
    }
}

/// 批次發佈多筆價格更新事件。
pub fn publish_price_updates(updates: Vec<(String, Decimal)>) {
    for (symbol, price) in updates {
        publish_price_update(symbol, price);
    }
}

/// 啟動價格更新事件 consumer。
fn start_price_update_consumer_task() {
    let mut tx_guard = match PRICE_UPDATE_TX.write() {
        Ok(guard) => guard,
        Err(why) => {
            logging::error_file_async(format!(
                "Failed to lock price update sender because {:?}",
                why
            ));
            return;
        }
    };

    if tx_guard.is_some() {
        return;
    }

    let (tx, mut rx) = mpsc::unbounded_channel::<PriceUpdateEvent>();
    *tx_guard = Some(tx);
    drop(tx_guard);

    task::spawn(async move {
        logging::info_file_async("股票追蹤價格事件 consumer 啟動".to_string());

        while let Some(event) = rx.recv().await {
            trace_stats::record_consumed_price_event();
            if let Err(why) = stock_price::evaluate_price_update(event.symbol, event.price).await {
                logging::error_file_async(format!(
                    "Failed to evaluate price update event because {:?}",
                    why
                ));
            }
        }

        logging::info_file_async("股票追蹤價格事件 consumer 已停止".to_string());
    });
}

/// 停止價格更新事件 consumer。
fn stop_price_update_consumer_task() {
    if let Ok(mut tx) = PRICE_UPDATE_TX.write() {
        tx.take();
    }
}

/// 啟動追蹤條件快取刷新任務。
fn start_trace_target_refresh_task() {
    if IS_TARGET_CACHE_REFRESHING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    task::spawn(async move {
        logging::info_file_async("追蹤條件快取刷新任務啟動".to_string());

        while IS_TARGET_CACHE_REFRESHING.load(Ordering::SeqCst) {
            time::sleep(TRACE_TARGET_REFRESH_INTERVAL).await;

            if !IS_TARGET_CACHE_REFRESHING.load(Ordering::SeqCst) {
                break;
            }

            match stock_price::refresh_trace_targets_cache().await {
                Ok(symbol_count) => {
                    logging::debug_file_async(format!(
                        "追蹤條件快取已刷新，共 {} 檔股票",
                        symbol_count
                    ));
                }
                Err(why) => {
                    logging::error_file_async(format!(
                        "Failed to refresh trace target cache because {:?}",
                        why
                    ));
                }
            }
        }

        IS_TARGET_CACHE_REFRESHING.store(false, Ordering::SeqCst);
        logging::info_file_async("追蹤條件快取刷新任務已停止".to_string());
    });
}

/// 停止追蹤條件快取刷新任務。
fn stop_trace_target_refresh_task() {
    IS_TARGET_CACHE_REFRESHING.store(false, Ordering::SeqCst);
}

/// 啟動低頻 reconciliation 任務。
fn start_trace_reconciliation_task() {
    if IS_RECONCILING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    task::spawn(async move {
        logging::info_file_async("追蹤股票低頻對帳任務啟動".to_string());
        let mut ticker = time::interval(TRACE_RECONCILIATION_INTERVAL);
        ticker.tick().await;

        while IS_RECONCILING.load(Ordering::SeqCst) {
            ticker.tick().await;

            if !IS_RECONCILING.load(Ordering::SeqCst) {
                break;
            }

            if !declare::StockExchange::TWSE.is_open() {
                break;
            }

            match stock_price::reconcile_target_prices().await {
                Ok(symbol_count) => {
                    trace_stats::record_reconciliation_run(symbol_count);
                }
                Err(why) => {
                    logging::error_file_async(format!(
                        "Failed to reconcile traced stock prices because {:?}",
                        why
                    ));
                }
            }
        }

        IS_RECONCILING.store(false, Ordering::SeqCst);
        logging::info_file_async("追蹤股票低頻對帳任務已停止".to_string());
    });
}

/// 停止低頻 reconciliation 任務。
fn stop_trace_reconciliation_task() {
    IS_RECONCILING.store(false, Ordering::SeqCst);
}

/// 啟動被追蹤股票的備援採集背景任務。
///
/// 此任務只採集 `Trace` 資料表中實際被追蹤的股票，並呼叫
/// [`crawler::fetch_stock_price_from_backup_sites`] 取得最新成交價。
/// 採集結果會以「單筆價格更新」方式寫回 `stock_snapshots`，
/// 若價格真的有異動，還會額外發佈價格更新事件，交由 trace evaluator 判斷是否通知。
fn start_traced_stock_backup_caching_task() {
    if IS_BACKUP_CACHING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    task::spawn(async move {
        logging::info_file_async("追蹤股票備援採集任務啟動".to_string());

        while IS_BACKUP_CACHING.load(Ordering::SeqCst) {
            if !declare::StockExchange::TWSE.is_open() {
                break;
            }

            if let Err(why) = refresh_traced_stock_snapshot_cache().await {
                logging::error_file_async(format!(
                    "Failed to refresh traced stock snapshot cache: {:?}",
                    why
                ));
            }

            if !IS_BACKUP_CACHING.load(Ordering::SeqCst) {
                break;
            }

            time::sleep(BACKUP_SNAPSHOT_REFRESH_INTERVAL).await;
        }

        IS_BACKUP_CACHING.store(false, Ordering::SeqCst);
        logging::info_file_async("追蹤股票備援採集任務已停止".to_string());
    });
}

/// 停止被追蹤股票的備援採集背景任務。
///
/// 此方法只會要求背景迴圈停止，不會直接清空共用的即時報價快取；
/// 快取清理仍交由 crawler 層的背景任務停止流程處理。
fn stop_traced_stock_backup_caching_task() {
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

    let results = future::join_all(
        symbols
            .into_iter()
            .map(|symbol| async move { refresh_single_traced_stock_snapshot(symbol).await }),
    )
    .await;

    let updated = results.into_iter().filter(|is_updated| *is_updated).count();
    logging::debug_file_async(format!("追蹤股票備援快取已更新 {} 檔", updated));

    Ok(())
}

/// 重新整理單一被追蹤股票的備援即時價格。
async fn refresh_single_traced_stock_snapshot(symbol: String) -> bool {
    match crawler::fetch_stock_price_from_backup_sites(&symbol).await {
        Ok(price) if price != Decimal::ZERO => {
            let previous_price = SHARE
                .get_stock_snapshot(&symbol)
                .map(|snapshot| snapshot.price);
            if previous_price == Some(price) {
                return false;
            }

            SHARE.set_stock_snapshot_price(symbol.clone(), price);
            publish_price_update(symbol, price);
            true
        }
        Ok(_) => {
            logging::debug_file_async(format!("Stock {} backup price is zero, skipping", symbol));
            false
        }
        Err(why) => {
            logging::error_file_async(format!(
                "Failed to fetch backup price for {}: {:?}",
                symbol, why
            ));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;
    use crate::database::table::trace::Trace;

    /// 將追蹤設定整理成不重複的股票代號清單。
    fn collect_traced_symbols(targets: Vec<Trace>) -> Vec<String> {
        let mut symbols = targets
            .into_iter()
            .map(|target| target.stock_symbol)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        symbols.sort();
        symbols
    }

    /// 驗證追蹤股票代號整理流程會去除重複值並依字串排序。
    #[test]
    fn test_collect_traced_symbols_deduplicates_and_sorts() {
        let symbols = collect_traced_symbols(vec![
            Trace::new("2330".to_string(), dec!(500), dec!(600)),
            Trace::new("2317".to_string(), dec!(100), dec!(120)),
            Trace::new("2330".to_string(), dec!(520), dec!(650)),
        ]);

        assert_eq!(symbols, vec!["2317".to_string(), "2330".to_string()]);
    }

    /// 驗證在未啟動 consumer 時發佈價格事件不會 panic。
    #[test]
    fn test_publish_price_update_without_consumer() {
        publish_price_update("2330".to_string(), dec!(998));
    }
}
