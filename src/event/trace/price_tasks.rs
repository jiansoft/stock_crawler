//! # Trace 即時價格任務協調模組
//!
//! 此模組負責管理 trace 事件所需的即時報價背景任務，
//! 讓 [`stock_price`](stock_price) 可以專注在
//! 價格追蹤與警報邏輯。
//!
//! 目前此模組會協調五種工作：
//! 1. crawler 層的全市場即時報價背景任務（目前由 Yahoo 類股快取驅動）
//! 2. 只針對 `Trace` 資料表內股票的備援採集任務
//! 3. 價格更新事件 consumer，將指定股票代號交給追蹤 evaluator
//! 4. 追蹤條件快取刷新任務，定期同步最新 `trace` 設定
//! 5. 低頻 reconciliation 任務，補償事件遺漏或剛新增追蹤條件的情況

use std::sync::RwLock;
use std::time::Duration;
use std::{
    collections::HashSet,
    mem::size_of,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    time::Instant,
};

use anyhow::Result;
use futures::future;
use once_cell::sync::Lazy;
use rust_decimal::Decimal;
use tokio::{
    sync::mpsc::{self, Sender},
    task, time,
};

use super::{stats as trace_stats, stock_price};
use crate::{
    cache::RealtimeSnapshot,
    util::diagnostics::{read_process_memory_stats, trim_allocator_memory, TaskRuntimeStatus},
};
use crate::{cache::SHARE, crawler, declare, logging};

/// 價格更新事件。
#[derive(Debug, Clone)]
struct PriceUpdateEvent {
    symbol: String,
}

/// 確保「被追蹤股票備援採集」只有一個背景任務在執行。
static IS_BACKUP_CACHING: AtomicBool = AtomicBool::new(false);
/// 確保「追蹤條件快取刷新」只有一個背景任務在執行。
static IS_TARGET_CACHE_REFRESHING: AtomicBool = AtomicBool::new(false);
/// 確保「低頻 reconciliation」只有一個背景任務在執行。
static IS_RECONCILING: AtomicBool = AtomicBool::new(false);
/// 確保「trace diagnostics 輸出」只有一個背景任務在執行。
static IS_DIAGNOSTICS_LOGGING: AtomicBool = AtomicBool::new(false);
/// 價格更新事件 sender；存在時代表 consumer 仍可接收事件。
static PRICE_UPDATE_TX: Lazy<RwLock<Option<Sender<PriceUpdateEvent>>>> =
    Lazy::new(|| RwLock::new(None));
/// 已排入 consumer 或正在處理中的股票代號，避免同一支股票在 queue 內無限堆積。
static PENDING_PRICE_SYMBOLS: Lazy<RwLock<HashSet<String>>> =
    Lazy::new(|| RwLock::new(HashSet::new()));
/// 價格事件 consumer 存活 task 數量。
static PRICE_CONSUMER_ACTIVE_TASKS: AtomicUsize = AtomicUsize::new(0);
/// 價格事件 consumer 最近一次啟動的世代編號。
static PRICE_CONSUMER_LAST_GENERATION: AtomicU64 = AtomicU64::new(0);
/// 追蹤條件快取刷新 task 存活數量。
static TARGET_REFRESH_ACTIVE_TASKS: AtomicUsize = AtomicUsize::new(0);
/// 追蹤條件快取刷新 task 最近一次啟動的世代編號。
static TARGET_REFRESH_LAST_GENERATION: AtomicU64 = AtomicU64::new(0);
/// reconciliation task 存活數量。
static RECONCILIATION_ACTIVE_TASKS: AtomicUsize = AtomicUsize::new(0);
/// reconciliation task 最近一次啟動的世代編號。
static RECONCILIATION_LAST_GENERATION: AtomicU64 = AtomicU64::new(0);
/// 備援抓價 task 存活數量。
static BACKUP_ACTIVE_TASKS: AtomicUsize = AtomicUsize::new(0);
/// 備援抓價 task 最近一次啟動的世代編號。
static BACKUP_LAST_GENERATION: AtomicU64 = AtomicU64::new(0);
/// diagnostics task 存活數量。
static DIAGNOSTICS_ACTIVE_TASKS: AtomicUsize = AtomicUsize::new(0);
/// diagnostics task 最近一次啟動的世代編號。
static DIAGNOSTICS_LAST_GENERATION: AtomicU64 = AtomicU64::new(0);
const SNAPSHOT_WARMUP_TIMEOUT: Duration = Duration::from_secs(3);
const SNAPSHOT_WARMUP_POLL_INTERVAL: Duration = Duration::from_millis(100);
const BACKUP_SNAPSHOT_REFRESH_INTERVAL: Duration = Duration::from_secs(15);
const TRACE_TARGET_REFRESH_INTERVAL: Duration = Duration::from_secs(60);
const TRACE_RECONCILIATION_INTERVAL: Duration = Duration::from_secs(60 * 5);
const TRACE_DIAGNOSTICS_LOG_INTERVAL: Duration = Duration::from_secs(30);
const TRACE_ALLOCATOR_TRIM_INTERVAL: Duration = Duration::from_secs(60 * 5);
const PRICE_UPDATE_CHANNEL_CAPACITY: usize = 4096;

fn decrement_active_tasks(counter: &AtomicUsize) -> usize {
    counter
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
            Some(current.saturating_sub(1))
        })
        .map(|previous| previous.saturating_sub(1))
        .unwrap_or_default()
}

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
    start_trace_diagnostics_task();
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
    stop_trace_diagnostics_task();
    stop_price_update_consumer_task();
    crawler::price_tasks::stop_price_tasks().await;
    stock_price::clear_trace_targets_cache();
    trace_stats::flush_runtime_stats();
}

/// 發佈單筆價格更新事件。
///
/// 事件只代表「這支股票的共享快取已更新」。
/// 實際追蹤比對時會重新從快取讀值，而不是直接使用這裡傳入的 `price`。
///
/// 若 trace 價格事件 consumer 尚未啟動或已停止，事件會直接被忽略。
pub fn publish_price_update(symbol: String, price: Decimal) {
    if price == Decimal::ZERO {
        return;
    }

    if !stock_price::has_targets_for_symbol(&symbol) {
        return;
    }

    let should_enqueue = match PENDING_PRICE_SYMBOLS.write() {
        Ok(mut pending) => pending.insert(symbol.clone()),
        Err(_) => {
            trace_stats::record_dropped_price_event();
            return;
        }
    };

    if !should_enqueue {
        return;
    }

    let tx = match PRICE_UPDATE_TX.read() {
        Ok(tx) => tx.as_ref().cloned(),
        Err(_) => None,
    };

    let send_result = tx.map(|tx| {
        tx.try_send(PriceUpdateEvent {
            symbol: symbol.clone(),
        })
        .is_ok()
    });

    if send_result == Some(true) {
        trace_stats::record_published_price_event();
        return;
    }

    clear_pending_price_symbol(&symbol);
    trace_stats::record_dropped_price_event();
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

    clear_pending_price_symbols();

    let (tx, mut rx) = mpsc::channel::<PriceUpdateEvent>(PRICE_UPDATE_CHANNEL_CAPACITY);
    *tx_guard = Some(tx);
    drop(tx_guard);
    let generation = PRICE_CONSUMER_LAST_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;

    task::spawn(async move {
        let active_tasks = PRICE_CONSUMER_ACTIVE_TASKS.fetch_add(1, Ordering::SeqCst) + 1;
        logging::info_file_async(format!(
            "股票追蹤價格事件 consumer 啟動 generation={} active_tasks={}",
            generation, active_tasks
        ));

        while let Some(event) = rx.recv().await {
            let symbol = event.symbol;
            trace_stats::record_consumed_price_event();
            // consumer 只通知「哪支股票剛更新」，
            // evaluator 會自行回頭讀共享快取來做高低標判斷。
            if let Err(why) = stock_price::evaluate_price_update(symbol.clone()).await {
                logging::error_file_async(format!(
                    "Failed to evaluate price update event because {:?}",
                    why
                ));
            }
            clear_pending_price_symbol(&symbol);
        }

        clear_pending_price_symbols();
        let active_tasks = decrement_active_tasks(&PRICE_CONSUMER_ACTIVE_TASKS);
        logging::info_file_async(format!(
            "股票追蹤價格事件 consumer 已停止 generation={} active_tasks={}",
            generation, active_tasks
        ));
    });
}

/// 停止價格更新事件 consumer。
fn stop_price_update_consumer_task() {
    if let Ok(mut tx) = PRICE_UPDATE_TX.write() {
        tx.take();
    }
    clear_pending_price_symbols();
}

/// 啟動追蹤條件快取刷新任務。
fn start_trace_target_refresh_task() {
    if IS_TARGET_CACHE_REFRESHING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let generation = TARGET_REFRESH_LAST_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;

    task::spawn(async move {
        let active_tasks = TARGET_REFRESH_ACTIVE_TASKS.fetch_add(1, Ordering::SeqCst) + 1;
        logging::info_file_async(format!(
            "追蹤條件快取刷新任務啟動 generation={} active_tasks={}",
            generation, active_tasks
        ));

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
        let active_tasks = decrement_active_tasks(&TARGET_REFRESH_ACTIVE_TASKS);
        logging::info_file_async(format!(
            "追蹤條件快取刷新任務已停止 generation={} active_tasks={}",
            generation, active_tasks
        ));
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
    let generation = RECONCILIATION_LAST_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;

    task::spawn(async move {
        let active_tasks = RECONCILIATION_ACTIVE_TASKS.fetch_add(1, Ordering::SeqCst) + 1;
        logging::info_file_async(format!(
            "追蹤股票低頻對帳任務啟動 generation={} active_tasks={}",
            generation, active_tasks
        ));
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
        let active_tasks = decrement_active_tasks(&RECONCILIATION_ACTIVE_TASKS);
        logging::info_file_async(format!(
            "追蹤股票低頻對帳任務已停止 generation={} active_tasks={}",
            generation, active_tasks
        ));
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
    let generation = BACKUP_LAST_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;

    task::spawn(async move {
        let active_tasks = BACKUP_ACTIVE_TASKS.fetch_add(1, Ordering::SeqCst) + 1;
        logging::info_file_async(format!(
            "追蹤股票備援採集任務啟動 generation={} active_tasks={}",
            generation, active_tasks
        ));

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
        let active_tasks = decrement_active_tasks(&BACKUP_ACTIVE_TASKS);
        logging::info_file_async(format!(
            "追蹤股票備援採集任務已停止 generation={} active_tasks={}",
            generation, active_tasks
        ));
    });
}

/// 停止被追蹤股票的備援採集背景任務。
///
/// 此方法只會要求背景迴圈停止，不會直接清空共用的即時報價快取；
/// 快取清理仍交由 crawler 層的背景任務停止流程處理。
fn stop_traced_stock_backup_caching_task() {
    IS_BACKUP_CACHING.store(false, Ordering::SeqCst);
}

/// 啟動 trace diagnostics 任務，定期輸出記憶體、快取與事件吞吐摘要。
fn start_trace_diagnostics_task() {
    if IS_DIAGNOSTICS_LOGGING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    let generation = DIAGNOSTICS_LAST_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;

    task::spawn(async move {
        let active_tasks = DIAGNOSTICS_ACTIVE_TASKS.fetch_add(1, Ordering::SeqCst) + 1;
        logging::info_file_async(format!(
            "trace diagnostics 任務啟動 generation={} active_tasks={}",
            generation, active_tasks
        ));

        let mut ticker = time::interval(TRACE_DIAGNOSTICS_LOG_INTERVAL);
        let mut previous_stats = trace_stats::get_runtime_stats_snapshot();
        let mut previous_logged_at = Instant::now();
        let mut previous_trimmed_at = Instant::now() - TRACE_ALLOCATOR_TRIM_INTERVAL;
        ticker.tick().await;

        while IS_DIAGNOSTICS_LOGGING.load(Ordering::SeqCst) {
            ticker.tick().await;

            if !IS_DIAGNOSTICS_LOGGING.load(Ordering::SeqCst) {
                break;
            }

            log_trace_diagnostics(
                &mut previous_stats,
                &mut previous_logged_at,
                &mut previous_trimmed_at,
            );
        }

        IS_DIAGNOSTICS_LOGGING.store(false, Ordering::SeqCst);
        let active_tasks = decrement_active_tasks(&DIAGNOSTICS_ACTIVE_TASKS);
        logging::info_file_async(format!(
            "trace diagnostics 任務已停止 generation={} active_tasks={}",
            generation, active_tasks
        ));
    });
}

/// 停止 trace diagnostics 任務。
fn stop_trace_diagnostics_task() {
    IS_DIAGNOSTICS_LOGGING.store(false, Ordering::SeqCst);
}

#[allow(unused_variables)]
fn log_trace_diagnostics(
    previous_stats: &mut trace_stats::TraceRuntimeStatsSnapshot,
    previous_logged_at: &mut Instant,
    previous_trimmed_at: &mut Instant,
) {
    let now = Instant::now();
    let elapsed = now.duration_since(*previous_logged_at);
    *previous_logged_at = now;

    let current_stats = trace_stats::get_runtime_stats_snapshot();
    let delta_published = current_stats
        .price_events_published
        .saturating_sub(previous_stats.price_events_published);
    let delta_consumed = current_stats
        .price_events_consumed
        .saturating_sub(previous_stats.price_events_consumed);
    let delta_dropped = current_stats
        .price_events_dropped
        .saturating_sub(previous_stats.price_events_dropped);
    *previous_stats = current_stats;

    let estimated_backlog = current_stats
        .price_events_published
        .saturating_sub(current_stats.price_events_consumed)
        .saturating_sub(current_stats.price_events_dropped);
    let pending_symbols = pending_price_symbols_len();

    let (snapshot_len, snapshot_capacity, snapshot_string_bytes, snapshot_reserved_bytes) =
        snapshot_cache_diagnostics();
    let target_diagnostics = stock_price::trace_target_diagnostics();
    let memory_stats = read_process_memory_stats();
    let histock_status = crawler::histock::price::diagnostics_snapshot();
    let histock_runtime = crawler::histock::price::runtime_diagnostics_snapshot();
    let yahoo_status = crawler::yahoo::price::diagnostics_snapshot();
    let yahoo_runtime = crawler::yahoo::price::runtime_diagnostics_snapshot();
    let consumer_status = price_consumer_status();
    let refresh_status = atomic_task_status(
        IS_TARGET_CACHE_REFRESHING.load(Ordering::SeqCst),
        &TARGET_REFRESH_ACTIVE_TASKS,
        &TARGET_REFRESH_LAST_GENERATION,
    );
    let reconciliation_status = atomic_task_status(
        IS_RECONCILING.load(Ordering::SeqCst),
        &RECONCILIATION_ACTIVE_TASKS,
        &RECONCILIATION_LAST_GENERATION,
    );
    let backup_status = atomic_task_status(
        IS_BACKUP_CACHING.load(Ordering::SeqCst),
        &BACKUP_ACTIVE_TASKS,
        &BACKUP_LAST_GENERATION,
    );
    let diagnostics_status = atomic_task_status(
        IS_DIAGNOSTICS_LOGGING.load(Ordering::SeqCst),
        &DIAGNOSTICS_ACTIVE_TASKS,
        &DIAGNOSTICS_LAST_GENERATION,
    );
    let default_log_status = logging::diagnostics_snapshot();
    let http_log_status = crate::util::http::diagnostics_snapshot();

    let memory_summary = memory_stats.map_or_else(
        || "rss=n/a vms=n/a".to_string(),
        |stats| {
            format!(
                "rss={:.1}MiB vms={:.1}MiB",
                kib_to_mib(stats.vm_rss_kib),
                kib_to_mib(stats.vm_size_kib)
            )
        },
    );

    let elapsed_secs = elapsed.as_secs_f64();
    let publish_rate = if elapsed_secs > 0.0 {
        delta_published as f64 / elapsed_secs
    } else {
        0.0
    };
    let consume_rate = if elapsed_secs > 0.0 {
        delta_consumed as f64 / elapsed_secs
    } else {
        0.0
    };

    /*
    logging::info_file_async(format!(
        "Trace diagnostics | {} | snapshots len={} cap={} strings={}KiB approx_reserved={:.1}MiB | targets symbols={} total={} | events pub={} cons={} drop={} backlog~={} pending={} delta_pub={} ({:.1}/s) delta_cons={} ({:.1}/s) delta_drop={} | tasks {} {} {} {} {} {} {} | logs default(q={}/{} drop={} proc={}) http(q={}/{} drop={} proc={})",
        memory_summary,
        snapshot_len,
        snapshot_capacity,
        snapshot_string_bytes / 1024,
        snapshot_reserved_bytes as f64 / (1024.0 * 1024.0),
        target_diagnostics.symbol_count,
        target_diagnostics.target_count,
        current_stats.price_events_published,
        current_stats.price_events_consumed,
        current_stats.price_events_dropped,
        estimated_backlog,
        pending_symbols,
        delta_published,
        publish_rate,
        delta_consumed,
        consume_rate,
        delta_dropped,
        format_task_status("histock", histock_status),
        format_task_status("yahoo", yahoo_status),
        format_task_status("consumer", consumer_status),
        format_task_status("refresh", refresh_status),
        format_task_status("reconcile", reconciliation_status),
        format_task_status("backup", backup_status),
        format_task_status("diag", diagnostics_status),
        default_log_status.queued_messages,
        default_log_status.channel_capacity,
        default_log_status.dropped_messages,
        default_log_status.processed_messages,
        http_log_status.queued_messages,
        http_log_status.channel_capacity,
        http_log_status.dropped_messages,
        http_log_status.processed_messages,
    ));

    logging::info_file_async(format!(
        "Trace source diagnostics | histock cycles={} body={}KiB rows={} snaps={} changed={} rss_delta={}KiB elapsed={}ms status={} | yahoo cycles={} ok={} fail={} pages={} raw_items={} snaps={} candidate={} rss_delta={}KiB elapsed={}ms status={}",
        histock_runtime.completed_cycles,
        histock_runtime.last_body_bytes / 1024,
        histock_runtime.last_row_count,
        histock_runtime.last_snapshot_count,
        histock_runtime.last_changed_event_count,
        format_signed_kib(histock_runtime.last_rss_delta_kib),
        histock_runtime.last_elapsed_ms,
        format_task_status("histock", histock_runtime.status),
        yahoo_runtime.completed_cycles,
        yahoo_runtime.last_success_count,
        yahoo_runtime.last_failure_count,
        yahoo_runtime.last_page_count,
        yahoo_runtime.last_raw_item_count,
        yahoo_runtime.last_snapshot_count,
        yahoo_runtime.last_candidate_event_count,
        format_signed_kib(yahoo_runtime.last_rss_delta_kib),
        yahoo_runtime.last_elapsed_ms,
        format_task_status("yahoo", yahoo_runtime.status),
    ));
    */

    maybe_trim_allocator(
        previous_trimmed_at,
        pending_symbols,
        estimated_backlog,
        default_log_status.queued_messages,
        http_log_status.queued_messages,
    );
}

fn maybe_trim_allocator(
    previous_trimmed_at: &mut Instant,
    pending_symbols: usize,
    estimated_backlog: u64,
    default_log_queued: usize,
    http_log_queued: usize,
) {
    if pending_symbols > 0 || estimated_backlog > 0 {
        return;
    }

    if default_log_queued > 0 || http_log_queued > 0 {
        return;
    }

    if previous_trimmed_at.elapsed() < TRACE_ALLOCATOR_TRIM_INTERVAL {
        return;
    }

    *previous_trimmed_at = Instant::now();

    if trim_allocator_memory() {
        logging::info_file_async(
            "Trace diagnostics | allocator trim requested after idle snapshot".to_string(),
        );
    }
}

fn snapshot_cache_diagnostics() -> (usize, usize, usize, usize) {
    SHARE
        .stock_snapshots
        .read()
        .map(|cache| {
            let len = cache.len();
            let capacity = cache.capacity();
            let string_bytes = cache.iter().fold(0usize, |acc, (symbol, snapshot)| {
                acc + symbol.len() + snapshot.symbol.len() + snapshot.name.len()
            });
            let reserved_bytes = capacity
                .saturating_mul(size_of::<(String, RealtimeSnapshot)>())
                .saturating_add(string_bytes);

            (len, capacity, string_bytes, reserved_bytes)
        })
        .unwrap_or_default()
}

fn price_consumer_status() -> TaskRuntimeStatus {
    let enabled = PRICE_UPDATE_TX
        .read()
        .map(|tx| tx.is_some())
        .unwrap_or(false);
    atomic_task_status(
        enabled,
        &PRICE_CONSUMER_ACTIVE_TASKS,
        &PRICE_CONSUMER_LAST_GENERATION,
    )
}

fn atomic_task_status(
    enabled: bool,
    active_tasks: &AtomicUsize,
    last_generation: &AtomicU64,
) -> TaskRuntimeStatus {
    TaskRuntimeStatus::new(
        enabled,
        active_tasks.load(Ordering::SeqCst),
        last_generation.load(Ordering::SeqCst),
    )
}

#[allow(dead_code)]
fn format_task_status(name: &str, status: TaskRuntimeStatus) -> String {
    format!(
        "{}(en={} active={} gen={})",
        name, status.enabled, status.active_tasks, status.last_generation
    )
}

fn kib_to_mib(kib: u64) -> f64 {
    kib as f64 / 1024.0
}

#[allow(dead_code)]
fn format_signed_kib(delta_kib: i64) -> String {
    if delta_kib >= 0 {
        format!("+{}", delta_kib)
    } else {
        delta_kib.to_string()
    }
}

fn clear_pending_price_symbol(symbol: &str) {
    if let Ok(mut pending) = PENDING_PRICE_SYMBOLS.write() {
        pending.remove(symbol);
    }
}

fn clear_pending_price_symbols() {
    if let Ok(mut pending) = PENDING_PRICE_SYMBOLS.write() {
        *pending = HashSet::new();
    }
}

fn pending_price_symbols_len() -> usize {
    PENDING_PRICE_SYMBOLS
        .read()
        .map(|pending| pending.len())
        .unwrap_or_default()
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
    // logging::debug_file_async(format!("追蹤股票備援快取已更新 {} 檔", updated));

    Ok(())
}

/// 重新整理單一被追蹤股票的備援即時價格。
async fn refresh_single_traced_stock_snapshot(symbol: String) -> bool {
    match crawler::fetch_stock_price_from_backup_sites_with_source(&symbol).await {
        Ok(result) if result.price != Decimal::ZERO => {
            let price = result.price;
            let source_site = result.site_name.to_string();
            let previous_snapshot = SHARE.get_stock_snapshot(&symbol);
            let price_changed = previous_snapshot
                .as_ref()
                .is_none_or(|snapshot| snapshot.price != price);
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
