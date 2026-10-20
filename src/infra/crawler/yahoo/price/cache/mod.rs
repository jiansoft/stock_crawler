//! # Yahoo 類股快取背景任務
//!
//! 此模組負責在開盤期間依序輪詢 Yahoo 的三大市場類股，
//! 並將解析後的報價快照整批寫回 [`SHARE`]。
//! 它的設計目標與 `histock::price` 類似，但資料來源改成 Yahoo 類股 API。
//!
//! - [`state`]：生命週期旗標與最近一輪診斷數據。
//! - [`apply`]：類股快照寫回共用快取並發佈價格事件。
//! - [`errors`]：WAF 阻擋與 5xx 暫時性錯誤。

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use rand::RngExt;
use tokio::time::sleep;

use crate::{
    core::util::{
        atomic::decrement_atomic_usize,
        diagnostics::{ProcessMemoryStats, read_process_memory_stats, trim_allocator_memory},
    },
    infra::{cache::SHARE, crawler::yahoo::YahooClassCategory},
};

use super::class_quote::{self, CategoryFetchResult};

/// 類股快照寫回共用快取。
mod apply;
/// 類股 API 錯誤分類。
mod errors;
/// 生命週期旗標與診斷數據。
mod state;

use apply::apply_category_snapshots;
use state::{
    ACTIVE_TASKS, CACHING_TASK, COMPLETED_CYCLES, IS_CACHING, LAST_TASK_GENERATION, rss_delta_kib,
    store_runtime_progress,
};
pub(crate) use state::{diagnostics_snapshot, runtime_diagnostics_snapshot};

/// 測試間共用的鎖：套用快照的測試與 live 啟停測試都會改動全域 [`SHARE`]。
#[cfg(test)]
use crate::infra::cache::REALTIME_TEST_LOCK as TEST_STATE_LOCK;

/// 全部類股輪詢完一輪後的休息時間。
const CYCLE_COOLDOWN: Duration = Duration::from_secs(5);

/// 長時間冷卻時檢查停止旗標的間隔。
const STOP_CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// 遭遇 Yahoo WAF 阻擋後，整輪輪詢的冷卻時間。
///
/// Yahoo 的阻擋是綁在來源端（IP / client）而非個別類股上，因此一旦被擋，
/// 換下一個類股再打只會再被擋一次。這個冷卻套用在**整輪**、只等一次，
/// 不是每個類股各等一次。
const DENIED_COOLDOWN: Duration = Duration::from_secs(600);

/// 分段睡滿指定時間，期間只要停止旗標被關掉就提前返回。
///
/// 十分鐘的冷卻若用單一 `sleep` 等待，服務停止最久要拖十分鐘才會生效。
/// 這裡切成 [`STOP_CHECK_INTERVAL`] 的小段，讓 stop 能及時中斷。
async fn sleep_while_caching(total: Duration) {
    let deadline = Instant::now() + total;

    while IS_CACHING.load(Ordering::SeqCst) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        sleep(remaining.min(STOP_CHECK_INTERVAL)).await;
    }
}

/// 一輪類股輪詢的統計。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct CycleStats {
    /// 成功的類股數。
    success: usize,
    /// 失敗的類股數。
    failure: usize,
    /// 抓取的分頁數。
    pages: usize,
    /// Yahoo 原始回應累積的 item 數。
    raw_items: usize,
    /// 價格異動、發佈 trace 事件的股票數。
    candidate_events: usize,
    /// 本輪是否遭遇 WAF 阻擋；遇到就中止剩餘類股。
    denied: bool,
}

/// 共用快取目前的股票數，只用於日誌與診斷。
fn cached_snapshot_count() -> usize {
    SHARE
        .stock_snapshots
        .read()
        .map(|cache| cache.len())
        .unwrap_or_default()
}

/// 把本輪目前的統計寫進診斷數據，回傳共用快取的股票數。
fn record_progress(
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
async fn run_cycle<'a, F, Fut>(
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
                    tracing::warn!(
                        "Yahoo 類股快取更新失敗（暫時性）: {} {}({}) {:#}",
                        category.exchange.label(),
                        category.name,
                        category.sector_id,
                        why
                    );
                } else {
                    tracing::error!(
                        "Yahoo 類股快取更新失敗: {} {}({}) {:?}",
                        category.exchange.label(),
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
            tracing::warn!(
                "Yahoo 採集遭遇 Request denied，中止本輪剩餘類股並進入冷卻。類股: {} {}({})",
                category.exchange.label(),
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
fn finish_cycle(
    stats: &CycleStats,
    cycle_started: Instant,
    cycle_memory_before: Option<ProcessMemoryStats>,
) -> Duration {
    // 讀共享快取目前總筆數，只拿來做可觀測性 log，不參與任何商業判斷。
    let total_count = cached_snapshot_count();

    // 若整輪結束後共享快取仍然是空的，代表本輪 Yahoo 採集沒有成功落地任何資料，
    // 這是一種需要人回頭檢查程式或來源格式的明確異常。
    if total_count == 0 {
        tracing::error!(
            "Yahoo 類股快取輪詢完成但沒有任何資料落地: success_count={} failure_count={} 耗時 {:?}",
            stats.success,
            stats.failure,
            cycle_started.elapsed()
        );
    }

    tracing::debug!(
        "Yahoo 類股快取輪詢完成，共 {} 檔股票，成功類股 {}，失敗類股 {}，candidate_events={}，耗時 {:?}",
        total_count,
        stats.success,
        stats.failure,
        stats.candidate_events,
        cycle_started.elapsed()
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

/// 啟動 Yahoo 類股快取背景任務。
///
/// 啟動後會：
/// 1. 依照既定順序走訪所有 Yahoo 類股分類。
/// 2. 每個類股抓完整分頁資料後，更新共用即時快取。
/// 3. 比對價格異動並發佈 trace 價格更新事件。
///
/// 若任務已經在執行中，重複呼叫不會再啟動第二條背景迴圈。
pub fn start_caching_task() {
    if let Ok(mut handle) = CACHING_TASK.lock()
        && handle.as_ref().is_some_and(|task| task.is_finished())
    {
        handle.take();
    }

    // 先擋掉重複啟動，避免同一時間跑出多條背景輪詢迴圈，
    // 導致互相覆寫快取、重複打 API 或重複發價格事件。
    //
    // 以 compare_exchange 原子地「檢查並占用」啟動權：只有一個呼叫者能把
    // false 換成 true，其餘並行呼叫者會失敗並直接返回。舊版「先 load 檢查、
    // 再 store 設定」不是原子操作，兩個並行呼叫者可能同時通過檢查並各自
    // 啟動一條背景迴圈（check-then-set 競態）。
    if IS_CACHING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let generation = LAST_TASK_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;

    // 真正的輪詢工作放到背景 task 執行，避免阻塞呼叫端。
    let handle = tokio::spawn(async move {
        // 類股清單在任務啟動時就先攤平成固定順序，
        // 讓每一輪巡檢的走訪順序穩定、可預期，也方便對照 log。
        let categories = class_quote::all_class_categories();
        // 這份 map 用來記住「每個類股上一輪有哪些股票」，
        // 這樣同一類股下一輪更新時，才能知道哪些舊股票應該從快取中移除。
        let mut category_symbols: HashMap<String, HashSet<String>> =
            HashMap::with_capacity(categories.len());

        let active_tasks = ACTIVE_TASKS.fetch_add(1, Ordering::SeqCst) + 1;
        tracing::info!(
            "Yahoo 類股快取任務啟動 generation={} active_tasks={}",
            generation,
            active_tasks
        );

        // 只要旗標還是 true，就持續一輪又一輪地輪詢所有類股。
        while IS_CACHING.load(Ordering::SeqCst) {
            // 記錄整輪開始時間與記憶體，讓診斷數據能看到一輪全部跑完花多久。
            let cycle_started = Instant::now();
            let cycle_memory_before = read_process_memory_stats();
            let stats = run_cycle(
                &categories,
                &mut category_symbols,
                cycle_started,
                cycle_memory_before,
                class_quote::fetch_category_snapshots,
            )
            .await;

            // 如果是在整輪尾端才收到 stop，就不要再進入 cooldown。
            if !IS_CACHING.load(Ordering::SeqCst) {
                break;
            }

            // 一輪全部類股跑完後稍作休息，避免無間斷全市場輪詢造成壓力過大。
            let cooldown = finish_cycle(&stats, cycle_started, cycle_memory_before);
            if stats.denied {
                sleep_while_caching(cooldown).await;
            } else {
                sleep(cooldown).await;
            }
        }

        // 跳出 while 代表旗標已關閉，這裡補一筆停止 log 方便對照啟停時間。
        IS_CACHING.store(false, Ordering::SeqCst);
        let active_tasks = decrement_atomic_usize(&ACTIVE_TASKS);
        tracing::info!(
            "Yahoo 類股快取任務已停止 generation={} active_tasks={}",
            generation,
            active_tasks
        );
    });

    if let Ok(mut task) = CACHING_TASK.lock() {
        *task = Some(handle);
    } else {
        tracing::error!(
            "{}",
            "Failed to store Yahoo caching task handle".to_string(),
        );
    }
}

/// 停止 Yahoo 類股快取背景任務並清空共用快取。
///
/// 此方法只負責將停止旗標設為 `false` 並清空 `stock_snapshots`。
/// 若有正在進行中的 HTTP 請求，背景迴圈會在該請求回來後檢查旗標，
/// 確保停止後不會再把資料寫回快取。
pub async fn stop_caching_task() {
    // 先關掉旗標，讓背景迴圈在下一個檢查點自行結束。
    IS_CACHING.store(false, Ordering::SeqCst);
    let handle = match CACHING_TASK.lock() {
        Ok(mut task) => task.take(),
        Err(why) => {
            tracing::error!("Failed to lock Yahoo caching task handle because {:?}", why);
            None
        }
    };

    if let Some(handle) = handle
        && let Err(why) = handle.await
    {
        tracing::error!("Yahoo 類股快取任務停止等待失敗: {:?}", why);
    }

    // 然後主動清空快取，避免收盤或停任務後外部仍讀到過期盤中報價。
    SHARE.clear_stock_snapshots();
}

#[cfg(test)]
mod tests;
