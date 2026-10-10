//! HiStock 全市場快取的背景任務與診斷資訊。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use rust_decimal::Decimal;
use tokio::task::JoinHandle;
use tokio::time::sleep;

use super::{
    FETCH_LOCK, HiStockFetchResult, fetch_all_from_rank,
    screen::{DirtyBatch, INCONSISTENT_ROW_WARNED},
};
use crate::{
    app::event::trace::price_tasks as trace_price_tasks,
    core::util::{
        atomic::decrement_atomic_usize,
        diagnostics::{
            ProcessMemoryStats, TaskRuntimeStatus, read_process_memory_stats, trim_allocator_memory,
        },
    },
    infra::cache::{RealtimeSnapshot, SHARE},
};

/// 全域快取狀態
pub(super) static IS_CACHING: Lazy<AtomicBool> = Lazy::new(|| AtomicBool::new(false));
/// 目前存活中的 HiStock 背景 task 數量。
pub(super) static ACTIVE_TASKS: Lazy<AtomicUsize> = Lazy::new(|| AtomicUsize::new(0));
/// HiStock 背景 task 的世代編號。
pub(super) static LAST_TASK_GENERATION: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(0));
/// 目前執行中的 HiStock 背景 task handle。
pub(super) static CACHING_TASK: Lazy<std::sync::Mutex<Option<JoinHandle<()>>>> =
    Lazy::new(|| std::sync::Mutex::new(None));
/// HiStock 最近一次抓取的 HTTP body 大小。
pub(super) static LAST_BODY_BYTES: Lazy<AtomicUsize> = Lazy::new(|| AtomicUsize::new(0));
/// HiStock 最近一次解析到的 HTML row 數量。
pub(super) static LAST_ROW_COUNT: Lazy<AtomicUsize> = Lazy::new(|| AtomicUsize::new(0));
/// HiStock 最近一次落地的快照數。
pub(super) static LAST_SNAPSHOT_COUNT: Lazy<AtomicUsize> = Lazy::new(|| AtomicUsize::new(0));
/// HiStock 最近一次產生的價格事件數。
pub(super) static LAST_CHANGED_EVENT_COUNT: Lazy<AtomicUsize> = Lazy::new(|| AtomicUsize::new(0));
/// HiStock 最近一次抓取耗時毫秒數。
pub(super) static LAST_ELAPSED_MS: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(0));
/// HiStock 最近一次 RSS 差值（KiB）。
pub(super) static LAST_RSS_DELTA_KIB: Lazy<AtomicI64> = Lazy::new(|| AtomicI64::new(0));
/// HiStock 完成抓取的累積輪數。
pub(super) static COMPLETED_CYCLES: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(0));

/// HiStock 來源最近一輪抓取的執行摘要。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct HiStockRuntimeDiagnostics {
    pub status: TaskRuntimeStatus,
    pub last_body_bytes: usize,
    pub last_row_count: usize,
    pub last_snapshot_count: usize,
    pub last_changed_event_count: usize,
    pub last_elapsed_ms: u64,
    pub last_rss_delta_kib: i64,
    pub completed_cycles: u64,
}

/// 比對新舊快取，收集「價格實際有異動」的股票清單。
///
/// # 回傳
/// - `Vec<(String, Decimal)>`：股票代號與最新成交價的配對。
///
/// # 行為
/// - 若舊快取中不存在該股票，視為新價格事件。
/// - 若舊快取中的 `price` 與新資料相同，則不產生事件。
/// - 價格為 0 的資料不會發出事件，避免無效資料觸發追蹤判斷。
pub(super) fn collect_changed_price_updates(
    new_data: &HashMap<String, RealtimeSnapshot>,
) -> Vec<(String, Decimal)> {
    let old_cache = SHARE.stock_snapshots.read().ok();
    let mut updates = Vec::new();

    for (symbol, snapshot) in new_data {
        if snapshot.price == Decimal::ZERO {
            continue;
        }

        let has_changed = old_cache
            .as_ref()
            .and_then(|cache| cache.get(symbol))
            .is_none_or(|old_snapshot| old_snapshot.price != snapshot.price);

        if has_changed {
            updates.push((symbol.clone(), snapshot.price));
        }
    }

    updates
}

/// 啟動定時快取任務。
///
/// 此任務會固定重新抓取 HiStock 全市場排行榜，並以全量覆蓋方式更新
/// [`SHARE`](infra::cache::SHARE) 的即時報價快取。
///
/// 若任務已在執行中，重複呼叫不會再額外啟動第二個背景迴圈。
pub fn start_caching_task() {
    if let Ok(mut handle) = CACHING_TASK.lock()
        && handle.as_ref().is_some_and(|task| task.is_finished())
    {
        handle.take();
    }

    // 以 compare_exchange 原子地「檢查並占用」啟動權：只有一個呼叫者能把
    // false 換成 true，其餘並行呼叫者會失敗並直接返回。
    // 舊版「先 load 檢查、再 store 設定」不是原子操作——兩個並行呼叫者
    // 可能同時通過檢查並各自啟動一條背景迴圈，造成重複抓取、快取互相
    // 覆寫與事件重複發布，且 CACHING_TASK 只會保存最後一個 JoinHandle。
    if IS_CACHING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let generation = LAST_TASK_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;

    let handle = tokio::spawn(async move {
        let active_tasks = ACTIVE_TASKS.fetch_add(1, Ordering::SeqCst) + 1;
        tracing::info!(
            "HiStock 全市場快取任務啟動 generation={} active_tasks={}",
            generation,
            active_tasks
        );

        while IS_CACHING.load(Ordering::SeqCst) {
            let start_time = Instant::now();
            let memory_before = read_process_memory_stats();

            // 背景任務也受 FETCH_LOCK 控制，但優先讓外部請求先行
            let result = {
                let _lock = FETCH_LOCK.lock().await;
                fetch_all_from_rank().await
            };

            match result {
                Ok(fetch_result) => record_cycle(fetch_result, start_time, memory_before),
                Err(e) => report_cycle_error(&e),
            }

            if !IS_CACHING.load(Ordering::SeqCst) {
                break;
            }
            sleep(Duration::from_secs(5)).await;
        }
        IS_CACHING.store(false, Ordering::SeqCst);
        let active_tasks = decrement_atomic_usize(&ACTIVE_TASKS);
        tracing::info!(
            "HiStock 快取任務已停止 generation={} active_tasks={}",
            generation,
            active_tasks
        );
    });

    if let Ok(mut task) = CACHING_TASK.lock() {
        *task = Some(handle);
    } else {
        tracing::error!(
            "{}",
            "Failed to store HiStock caching task handle".to_string(),
        );
    }
}

/// 套用一輪成功的抓取結果：覆蓋快取、發布價格異動事件並更新診斷計數器。
///
/// 從背景迴圈拆出，不碰網路，單元測試可以直接餵組好的抓取結果。
pub(super) fn record_cycle(
    fetch_result: HiStockFetchResult,
    start_time: Instant,
    memory_before: Option<ProcessMemoryStats>,
) {
    let count = fetch_result.snapshots.len();
    let row_count = fetch_result.row_count;
    let body_bytes = fetch_result.body_bytes;
    let price_updates = collect_changed_price_updates(&fetch_result.snapshots);
    let changed_events = price_updates.len();
    SHARE.set_stock_snapshots(fetch_result.snapshots);
    trace_price_tasks::publish_price_updates(price_updates);
    let elapsed_ms = start_time.elapsed().as_millis().min(u64::MAX as u128) as u64;
    let _rss_delta_before_trim_kib = rss_delta_kib(memory_before, read_process_memory_stats());
    let _allocator_trimmed = trim_allocator_memory();

    let rss_delta_kib = rss_delta_kib(memory_before, read_process_memory_stats());
    LAST_BODY_BYTES.store(body_bytes, Ordering::SeqCst);
    LAST_ROW_COUNT.store(row_count, Ordering::SeqCst);
    LAST_SNAPSHOT_COUNT.store(count, Ordering::SeqCst);
    LAST_CHANGED_EVENT_COUNT.store(changed_events, Ordering::SeqCst);
    LAST_ELAPSED_MS.store(elapsed_ms, Ordering::SeqCst);
    LAST_RSS_DELTA_KIB.store(rss_delta_kib, Ordering::SeqCst);
    COMPLETED_CYCLES.fetch_add(1, Ordering::SeqCst);
    /*
    crate::tracing::debug!("HiStock 快取已更新，共 {} 檔股票，rows={} body={}KiB changed_events={} rss_delta={}KiB，耗時 {:?}",
        count,
        row_count,
        body_bytes / 1024,
        changed_events,
        rss_delta_kib,
        start_time.elapsed());
    */
    /*
    crate::tracing::info!("HiStock cycle diagnostics | snapshots={} rows={} body={}KiB changed_events={} rss_delta={}KiB rss_delta_before_trim={}KiB trim={} elapsed={}ms",
        count,
        row_count,
        body_bytes / 1024,
        changed_events,
        rss_delta_kib,
        rss_delta_before_trim_kib,
        allocator_trimmed,
        elapsed_ms,);
    */
}

/// 記錄一輪失敗的抓取：整批錯亂每天只記第一筆 warn，其他錯誤記 error。
pub(super) fn report_cycle_error(e: &anyhow::Error) {
    if e.downcast_ref::<DirtyBatch>().is_some() {
        // 錯亂通常會連續好幾輪（每 5 秒一輪），每天只記第一筆 warn。
        let first = INCONSISTENT_ROW_WARNED
            .lock()
            .map(|mut seen| seen.first_today("*dirty-batch*", chrono::Local::now().date_naive()))
            .unwrap_or(true);
        if first {
            tracing::warn!("{}（今天不再重複記錄）", e);
        } else {
            tracing::debug!("{}", e);
        }
    } else {
        tracing::error!("HiStock 快取更新失敗: {:?}", e);
    }
}

/// 停止定時快取任務並清空即時報價快取。
///
/// 清空快取的目的，是避免收盤後保留過時盤中資料，讓下次開盤重新暖機時
/// 一定從新資料開始。
pub async fn stop_caching_task() {
    IS_CACHING.store(false, Ordering::SeqCst);
    let handle = match CACHING_TASK.lock() {
        Ok(mut task) => task.take(),
        Err(why) => {
            tracing::error!(
                "Failed to lock HiStock caching task handle because {:?}",
                why
            );
            None
        }
    };

    if let Some(handle) = handle
        && let Err(why) = handle.await
    {
        tracing::error!("HiStock 快取任務停止等待失敗: {:?}", why);
    }

    SHARE.clear_stock_snapshots();
}

/// 取得 HiStock 背景任務目前的執行狀態。
pub(crate) fn diagnostics_snapshot() -> TaskRuntimeStatus {
    TaskRuntimeStatus::new(
        IS_CACHING.load(Ordering::SeqCst),
        ACTIVE_TASKS.load(Ordering::SeqCst),
        LAST_TASK_GENERATION.load(Ordering::SeqCst),
    )
}

/// 取得 HiStock 最近一輪抓取的執行摘要。
pub(crate) fn runtime_diagnostics_snapshot() -> HiStockRuntimeDiagnostics {
    HiStockRuntimeDiagnostics {
        status: diagnostics_snapshot(),
        last_body_bytes: LAST_BODY_BYTES.load(Ordering::SeqCst),
        last_row_count: LAST_ROW_COUNT.load(Ordering::SeqCst),
        last_snapshot_count: LAST_SNAPSHOT_COUNT.load(Ordering::SeqCst),
        last_changed_event_count: LAST_CHANGED_EVENT_COUNT.load(Ordering::SeqCst),
        last_elapsed_ms: LAST_ELAPSED_MS.load(Ordering::SeqCst),
        last_rss_delta_kib: LAST_RSS_DELTA_KIB.load(Ordering::SeqCst),
        completed_cycles: COMPLETED_CYCLES.load(Ordering::SeqCst),
    }
}

pub(super) fn rss_delta_kib(
    before: Option<ProcessMemoryStats>,
    after: Option<ProcessMemoryStats>,
) -> i64 {
    match (before, after) {
        (Some(before), Some(after)) => after.vm_rss_kib as i64 - before.vm_rss_kib as i64,
        _ => 0,
    }
}
