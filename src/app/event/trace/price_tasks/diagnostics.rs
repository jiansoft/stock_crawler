//! trace 背景任務的定期診斷：事件吞吐、快取大小、各 task 狀態，以及閒置時的記憶體整理。

use std::{
    mem::size_of,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
    time::{Duration, Instant},
};

use tokio::{task, time};

use super::{
    BACKUP_ACTIVE_TASKS, BACKUP_LAST_GENERATION, DIAGNOSTICS_ACTIVE_TASKS,
    DIAGNOSTICS_LAST_GENERATION, IS_BACKUP_CACHING, IS_DIAGNOSTICS_LOGGING, IS_RECONCILING,
    IS_TARGET_CACHE_REFRESHING, PRICE_CONSUMER_ACTIVE_TASKS, PRICE_CONSUMER_LAST_GENERATION,
    PRICE_UPDATE_TX, RECONCILIATION_ACTIVE_TASKS, RECONCILIATION_LAST_GENERATION,
    TARGET_REFRESH_ACTIVE_TASKS, TARGET_REFRESH_LAST_GENERATION, TRACE_TASK_STOP_NOTIFY,
    pending_price_symbols_len,
};
use crate::{
    app::event::trace::{stats as trace_stats, stock_price},
    core::logging,
    core::util::{
        atomic::decrement_atomic_usize,
        diagnostics::{TaskRuntimeStatus, read_process_memory_stats, trim_allocator_memory},
    },
    infra::cache::{RealtimeSnapshot, SHARE},
};

const TRACE_DIAGNOSTICS_LOG_INTERVAL: Duration = Duration::from_secs(30);
const TRACE_ALLOCATOR_TRIM_INTERVAL: Duration = Duration::from_secs(60 * 5);

/// 啟動 trace diagnostics 任務，定期輸出記憶體、快取與事件吞吐摘要。
pub(super) fn start_trace_diagnostics_task() {
    if IS_DIAGNOSTICS_LOGGING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    let generation = DIAGNOSTICS_LAST_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;

    task::spawn(async move {
        let active_tasks = DIAGNOSTICS_ACTIVE_TASKS.fetch_add(1, Ordering::SeqCst) + 1;
        tracing::info!(
            "trace diagnostics 任務啟動 generation={} active_tasks={}",
            generation,
            active_tasks
        );

        let mut ticker = time::interval(TRACE_DIAGNOSTICS_LOG_INTERVAL);
        let mut previous_stats = trace_stats::get_runtime_stats_snapshot();
        let mut previous_logged_at = Instant::now();
        let mut previous_trimmed_at = Instant::now() - TRACE_ALLOCATOR_TRIM_INTERVAL;
        ticker.tick().await;

        while IS_DIAGNOSTICS_LOGGING.load(Ordering::SeqCst) {
            tokio::select! {
                _ = ticker.tick() => {}
                _ = TRACE_TASK_STOP_NOTIFY.notified() => {}
            }

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
        let active_tasks = decrement_atomic_usize(&DIAGNOSTICS_ACTIVE_TASKS);
        tracing::info!(
            "trace diagnostics 任務已停止 generation={} active_tasks={}",
            generation,
            active_tasks
        );
    });
}

/// 停止 trace diagnostics 任務。
pub(super) fn stop_trace_diagnostics_task() {
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
    let histock_status = crate::infra::crawler::histock::price::diagnostics_snapshot();
    let histock_runtime = crate::infra::crawler::histock::price::runtime_diagnostics_snapshot();
    let yahoo_status = crate::infra::crawler::yahoo::price::diagnostics_snapshot();
    let yahoo_runtime = crate::infra::crawler::yahoo::price::runtime_diagnostics_snapshot();
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
    let http_log_status = crate::core::util::http::diagnostics_snapshot();

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
    tracing::info!("Trace diagnostics | {} | snapshots len={} cap={} strings={}KiB approx_reserved={:.1}MiB | targets symbols={} total={} | events pub={} cons={} drop={} backlog~={} pending={} delta_pub={} ({:.1}/s) delta_cons={} ({:.1}/s) delta_drop={} | tasks {} {} {} {} {} {} {} | logs default(q={}/{} drop={} proc={}) http(q={}/{} drop={} proc={})",
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
        http_log_status.processed_messages,);

    tracing::info!("Trace source diagnostics | histock cycles={} body={}KiB rows={} snaps={} changed={} rss_delta={}KiB elapsed={}ms status={} | yahoo cycles={} ok={} fail={} pages={} raw_items={} snaps={} candidate={} rss_delta={}KiB elapsed={}ms status={}",
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
        format_task_status("yahoo", yahoo_runtime.status),);
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
        tracing::info!(
            "{}",
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
