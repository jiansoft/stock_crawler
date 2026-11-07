//! 一輪類股輪詢：依序抓類股、寫回共用快取，並在結束時記錄摘要與決定冷卻時間。

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use rand::RngExt;
use tokio::time::sleep;

use crate::{
    core::util::diagnostics::{
        ProcessMemoryStats, read_process_memory_stats, trim_allocator_memory,
    },
    infra::{cache::SHARE, crawler::yahoo::YahooClassCategory},
};

use super::super::class_quote::CategoryFetchResult;
use super::apply::apply_category_snapshots;
use super::errors;
use super::state::{COMPLETED_CYCLES, IS_CACHING, rss_delta_kib, store_runtime_progress};
use super::{CYCLE_COOLDOWN, DENIED_COOLDOWN};

/// 一輪類股輪詢的統計。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct CycleStats {
    /// 成功的類股數。
    pub(super) success: usize,
    /// 失敗的類股數。
    pub(super) failure: usize,
    /// 抓取的分頁數。
    pub(super) pages: usize,
    /// Yahoo 原始回應累積的 item 數。
    pub(super) raw_items: usize,
    /// 價格異動、發佈 trace 事件的股票數。
    pub(super) candidate_events: usize,
    /// 本輪是否遭遇 WAF 阻擋；遇到就中止剩餘類股。
    pub(super) denied: bool,
}

/// 共用快取目前的股票數，只用於日誌與診斷。
pub(super) fn cached_snapshot_count() -> usize {
    SHARE
        .stock_snapshots
        .read()
        .map(|cache| cache.len())
        .unwrap_or_default()
}

/// 把本輪目前的統計寫進診斷數據，回傳共用快取的股票數。
pub(super) fn record_progress(
    stats: &CycleStats,
    cycle_started: Instant,
    cycle_memory_before: Option<ProcessMemoryStats>,
) -> usize {
    let total_count = cached_snapshot_count();
    store_runtime_progress(
        stats.success,
        stats.failure,
        stats.pages,
        stats.raw_items,
        total_count,
        stats.candidate_events,
        cycle_started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        rss_delta_kib(cycle_memory_before, read_process_memory_stats()),
    );
    total_count
}

/// 依固定順序抓完一輪類股並寫回共用快取。
///
/// 停止旗標關閉（含 HTTP 請求進行中被停止）或遭遇 WAF 阻擋時提前結束；
/// `fetch` 是類股抓取函式，測試時換成假資料。
pub(super) async fn run_cycle<'a, F, Fut>(
    categories: &[&'a YahooClassCategory],
    category_symbols: &mut HashMap<String, HashSet<String>>,
    cycle_started: Instant,
    cycle_memory_before: Option<ProcessMemoryStats>,
    fetch: F,
) -> CycleStats
where
    F: Fn(&'a YahooClassCategory) -> Fut,
    Fut: Future<Output = anyhow::Result<CategoryFetchResult>>,
{
    let mut stats = CycleStats::default();

    for &category in categories {
        // 在每個類股開始前先檢查一次停止旗標，
        // 避免外部要求停止後還繼續多跑好幾個類股。
        if !IS_CACHING.load(Ordering::SeqCst) {
            break;
        }

        // 類股抓取本身可能要跨多頁，所以這裡把整個類股抓完整再回來。
        let fetch_result = fetch(category).await;

        // 這個檢查非常重要：
        // 如果 stop 發生在 HTTP request 進行中，這裡能阻止「請求回來後又把資料寫回快取」。
        if !IS_CACHING.load(Ordering::SeqCst) {
            break;
        }

        match fetch_result {
            Ok(category_result) => {
                stats.success += 1;
                stats.pages += category_result.diagnostics.page_count;
                stats.raw_items += category_result.diagnostics.raw_item_count;
                // 這一步會把舊股票移除、把新股票寫進共享快取，
                // 同時完成價格異動計數與事件發佈。
                stats.candidate_events +=
                    apply_category_snapshots(category, category_result.snapshots, category_symbols)
                        .changed_event_count;
            }
            Err(why) => {
                stats.failure += 1;
                let err_msg = why.to_string();
                // 檢查錯誤訊息是否包含被阻擋特徵
                if errors::is_denied(&err_msg) {
                    stats.denied = true;
                    errors::alert_denied_once(category, &err_msg);
                }
                // 類股失敗時只記錄錯誤，不中止整輪任務，
                // 避免單一 sector 出問題就拖垮整個 Yahoo 報價快取。
                // 5xx 是 Yahoo 端的暫時性錯誤，下一輪即恢復，只記 warn。
                if errors::is_transient_server_error(&err_msg) {
                    let exchange_label = category.exchange.label();
                    tracing::warn!(
                        "Yahoo 類股快取更新失敗（暫時性）: {} {}({}) {:#}",
                        exchange_label,
                        category.name,
                        category.sector_id,
                        why
                    );
                } else {
                    let exchange_label = category.exchange.label();
                    tracing::error!(
                        "Yahoo 類股快取更新失敗: {} {}({}) {:?}",
                        exchange_label,
                        category.name,
                        category.sector_id,
                        why
                    );
                }
            }
        }

        // 每個類股處理完就歸還配置器記憶體，並更新診斷數據。
        trim_allocator_memory();
        record_progress(&stats, cycle_started, cycle_memory_before);

        // 類股處理完後再檢查一次停止旗標，
        // 讓 stop 能在兩個類股之間盡快生效。
        if !IS_CACHING.load(Ordering::SeqCst) {
            break;
        }

        // 遭遇 WAF 阻擋就直接中止本輪：阻擋綁在來源端而非個別類股，
        // 繼續往下跑只會對每個剩餘類股各再被擋一次。
        // 冷卻改由輪尾統一等一次（DENIED_COOLDOWN），不在這裡等。
        if stats.denied {
            let exchange_label = category.exchange.label();
            tracing::warn!(
                "Yahoo 採集遭遇 Request denied，中止本輪剩餘類股並進入冷卻。類股: {} {}({})",
                exchange_label,
                category.name,
                category.sector_id
            );
            break;
        }

        // 類股與類股之間進行隨機 2.0 至 4.0 秒的延遲（Jitter），降低規律請求被 Yahoo WAF 偵測為爬蟲的機率。
        let jitter_ms = rand::rng().random_range(2000..=4000);
        sleep(Duration::from_millis(jitter_ms)).await;
    }

    stats
}

/// 整輪結束：記錄摘要與診斷數據、累計完成輪數，回傳下一輪前的冷卻時間。
///
/// 本輪若曾被 WAF 擋下，改用較長的冷卻，讓來源端的封鎖有時間退場。
pub(super) fn finish_cycle(
    stats: &CycleStats,
    cycle_started: Instant,
    cycle_memory_before: Option<ProcessMemoryStats>,
) -> Duration {
    // 讀共享快取目前總筆數，只拿來做可觀測性 log，不參與任何商業判斷。
    let total_count = cached_snapshot_count();

    // 若整輪結束後共享快取仍然是空的，代表本輪 Yahoo 採集沒有成功落地任何資料，
    // 這是一種需要人回頭檢查程式或來源格式的明確異常。
    if total_count == 0 {
        let cycle_elapsed = cycle_started.elapsed();
        tracing::error!(
            "Yahoo 類股快取輪詢完成但沒有任何資料落地: success_count={} failure_count={} 耗時 {:?}",
            stats.success,
            stats.failure,
            cycle_elapsed
        );
    }

    let cycle_elapsed = cycle_started.elapsed();
    tracing::debug!(
        "Yahoo 類股快取輪詢完成，共 {} 檔股票，成功類股 {}，失敗類股 {}，candidate_events={}，耗時 {:?}",
        total_count,
        stats.success,
        stats.failure,
        stats.candidate_events,
        cycle_elapsed
    );

    trim_allocator_memory();
    record_progress(stats, cycle_started, cycle_memory_before);
    COMPLETED_CYCLES.fetch_add(1, Ordering::SeqCst);

    if stats.denied {
        tracing::warn!(
            "Yahoo 本輪遭遇 Request denied，冷卻 {:?} 後再重新輪詢。",
            DENIED_COOLDOWN
        );
        DENIED_COOLDOWN
    } else {
        CYCLE_COOLDOWN
    }
}
