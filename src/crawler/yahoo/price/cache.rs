//! # Yahoo 類股快取背景任務
//!
//! 此模組負責在開盤期間依序輪詢 Yahoo 的三大市場類股，
//! 並將解析後的報價快照整批寫回 [`SHARE`](crate::cache::SHARE)。
//! 它的設計目標與 `histock::price` 類似，但資料來源改成 Yahoo 類股 API。

use std::{
    collections::{HashMap, HashSet},
    sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering},
    time::{Duration, Instant},
};

use once_cell::sync::Lazy;
use rust_decimal::Decimal;
use tokio::task::JoinHandle;
use tokio::time::sleep;

use crate::{
    cache::{RealtimeSnapshot, SHARE},
    crawler::yahoo::YahooClassCategory,
    event::trace::price_tasks as trace_price_tasks,
    util::diagnostics::{
        read_process_memory_stats, trim_allocator_memory, ProcessMemoryStats, TaskRuntimeStatus,
    },
};

use super::class_quote;

/// 控制 Yahoo 類股快取背景任務生命週期的全域旗標。
static IS_CACHING: Lazy<AtomicBool> = Lazy::new(|| AtomicBool::new(false));
/// 目前存活中的 Yahoo 類股背景 task 數量。
static ACTIVE_TASKS: Lazy<AtomicUsize> = Lazy::new(|| AtomicUsize::new(0));
/// Yahoo 類股背景 task 的世代編號。
static LAST_TASK_GENERATION: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(0));
/// 目前執行中的 Yahoo 類股背景 task handle。
static CACHING_TASK: Lazy<std::sync::Mutex<Option<JoinHandle<()>>>> =
    Lazy::new(|| std::sync::Mutex::new(None));
/// Yahoo 最近一輪成功輪詢的類股數。
static LAST_SUCCESS_COUNT: Lazy<AtomicUsize> = Lazy::new(|| AtomicUsize::new(0));
/// Yahoo 最近一輪失敗的類股數。
static LAST_FAILURE_COUNT: Lazy<AtomicUsize> = Lazy::new(|| AtomicUsize::new(0));
/// Yahoo 最近一輪抓取的總頁數。
static LAST_PAGE_COUNT: Lazy<AtomicUsize> = Lazy::new(|| AtomicUsize::new(0));
/// Yahoo 最近一輪讀到的原始 item 總數。
static LAST_RAW_ITEM_COUNT: Lazy<AtomicUsize> = Lazy::new(|| AtomicUsize::new(0));
/// Yahoo 最近一輪落地後的總快取筆數。
static LAST_SNAPSHOT_COUNT: Lazy<AtomicUsize> = Lazy::new(|| AtomicUsize::new(0));
/// Yahoo 最近一輪候選價格事件數。
static LAST_CANDIDATE_EVENT_COUNT: Lazy<AtomicUsize> = Lazy::new(|| AtomicUsize::new(0));
/// Yahoo 最近一輪整體耗時毫秒數。
static LAST_ELAPSED_MS: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(0));
/// Yahoo 最近一輪 RSS 差值（KiB）。
static LAST_RSS_DELTA_KIB: Lazy<AtomicI64> = Lazy::new(|| AtomicI64::new(0));
/// Yahoo 完成輪詢的累積輪數。
static COMPLETED_CYCLES: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(0));

/// 相鄰兩個類股請求之間的節流間隔。
const CATEGORY_REQUEST_INTERVAL: Duration = Duration::from_secs(2);
/// 全部類股輪詢完一輪後的休息時間。
const CYCLE_COOLDOWN: Duration = Duration::from_secs(5);

/// Yahoo 類股來源的最近一輪執行摘要。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct YahooRuntimeDiagnostics {
    pub status: TaskRuntimeStatus,
    pub last_success_count: usize,
    pub last_failure_count: usize,
    pub last_page_count: usize,
    pub last_raw_item_count: usize,
    pub last_snapshot_count: usize,
    pub last_candidate_event_count: usize,
    pub last_elapsed_ms: u64,
    pub last_rss_delta_kib: i64,
    pub completed_cycles: u64,
}

fn decrement_active_tasks(counter: &AtomicUsize) -> usize {
    counter
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
            Some(current.saturating_sub(1))
        })
        .map(|previous| previous.saturating_sub(1))
        .unwrap_or_default()
}

fn store_runtime_progress(
    success_count: usize,
    failure_count: usize,
    page_count: usize,
    raw_item_count: usize,
    total_snapshot_count: usize,
    candidate_event_count: usize,
    elapsed_ms: u64,
    rss_delta_kib: i64,
) {
    LAST_SUCCESS_COUNT.store(success_count, Ordering::SeqCst);
    LAST_FAILURE_COUNT.store(failure_count, Ordering::SeqCst);
    LAST_PAGE_COUNT.store(page_count, Ordering::SeqCst);
    LAST_RAW_ITEM_COUNT.store(raw_item_count, Ordering::SeqCst);
    LAST_SNAPSHOT_COUNT.store(total_snapshot_count, Ordering::SeqCst);
    LAST_CANDIDATE_EVENT_COUNT.store(candidate_event_count, Ordering::SeqCst);
    LAST_ELAPSED_MS.store(elapsed_ms, Ordering::SeqCst);
    LAST_RSS_DELTA_KIB.store(rss_delta_kib, Ordering::SeqCst);
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
    if let Ok(mut handle) = CACHING_TASK.lock() {
        if handle.as_ref().is_some_and(|task| task.is_finished()) {
            handle.take();
        }
    }

    // 先擋掉重複啟動，避免同一時間跑出多條背景輪詢迴圈，
    // 導致互相覆寫快取、重複打 API 或重複發價格事件。
    if IS_CACHING.load(Ordering::SeqCst) {
        return;
    }
    // 一旦決定啟動，就先把旗標設為 true，後面任何其他呼叫者都會看到任務已啟動。
    IS_CACHING.store(true, Ordering::SeqCst);
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
        crate::logging::info_file_async(format!(
            "Yahoo 類股快取任務啟動 generation={} active_tasks={}",
            generation, active_tasks
        ));

        // 只要旗標還是 true，就持續一輪又一輪地輪詢所有類股。
        while IS_CACHING.load(Ordering::SeqCst) {
            // 記錄整輪開始時間，讓 log 能看到一輪全部跑完花多久。
            let cycle_started = Instant::now();
            let cycle_memory_before = read_process_memory_stats();
            // 記錄本輪成功與失敗的類股數，方便從整輪摘要看出採集是否異常。
            let mut success_count = 0usize;
            let mut failure_count = 0usize;
            let mut candidate_event_count = 0usize;
            let mut page_count = 0usize;
            let mut raw_item_count = 0usize;

            // 依固定順序逐類股更新，這樣比較容易控制節流與追蹤問題類股。
            for category in &categories {
                // 在每個類股開始前先檢查一次停止旗標，
                // 避免外部要求停止後還繼續多跑好幾個類股。
                if !IS_CACHING.load(Ordering::SeqCst) {
                    break;
                }

                // 單一類股的耗時獨立計算，方便從 log 看出是哪個類股變慢。
                let started_at = Instant::now();
                let category_memory_before = read_process_memory_stats();
                // 類股抓取本身可能要跨多頁，所以這裡把整個類股抓完整再回來。
                let fetch_result = class_quote::fetch_category_snapshots(category).await;

                // 這個檢查非常重要：
                // 如果 stop 發生在 HTTP request 進行中，這裡能阻止「請求回來後又把資料寫回快取」。
                if !IS_CACHING.load(Ordering::SeqCst) {
                    break;
                }

                match fetch_result {
                    Ok(category_result) => {
                        success_count += 1;
                        page_count += category_result.diagnostics.page_count;
                        raw_item_count += category_result.diagnostics.raw_item_count;
                        // 先記下本類股股票數，後面 log 會拿來判讀資料完整度。
                        let stock_count = category_result.diagnostics.snapshot_count;
                        // 這一步會把舊股票移除、把新股票寫進共享快取，
                        // 同時完成價格異動計數與事件發佈。
                        let apply_result = apply_category_snapshots(
                            category,
                            category_result.snapshots,
                            &mut category_symbols,
                        );
                        let changed_events = apply_result.changed_event_count;
                        candidate_event_count += changed_events;
                        let total_count = apply_result.total_snapshot_count;
                        let category_rss_delta_before_trim_kib =
                            rss_delta_kib(category_memory_before, read_process_memory_stats());
                        let category_trimmed = trim_allocator_memory();
                        let category_rss_delta_kib =
                            rss_delta_kib(category_memory_before, read_process_memory_stats());
                        let cycle_elapsed_ms =
                            cycle_started.elapsed().as_millis().min(u64::MAX as u128) as u64;
                        let cycle_rss_delta_kib =
                            rss_delta_kib(cycle_memory_before, read_process_memory_stats());
                        store_runtime_progress(
                            success_count,
                            failure_count,
                            page_count,
                            raw_item_count,
                            total_count,
                            candidate_event_count,
                            cycle_elapsed_ms,
                            cycle_rss_delta_kib,
                        );
                        crate::logging::info_file_async(format!(
                            "Yahoo category diagnostics | {} {}({}) snaps={} total={} pages={} raw_items={} changed_events={} rss_delta={}KiB rss_delta_before_trim={}KiB trim={} elapsed={}ms",
                            category.exchange.label(),
                            category.name,
                            category.sector_id,
                            stock_count,
                            total_count,
                            category_result.diagnostics.page_count,
                            category_result.diagnostics.raw_item_count,
                            changed_events,
                            category_rss_delta_kib,
                            category_rss_delta_before_trim_kib,
                            category_trimmed,
                            started_at.elapsed().as_millis().min(u64::MAX as u128) as u64,
                        ));
                    }
                    Err(why) => {
                        failure_count += 1;
                        let total_count = SHARE
                            .stock_snapshots
                            .read()
                            .map(|cache| cache.len())
                            .unwrap_or_default();
                        let category_rss_delta_before_trim_kib =
                            rss_delta_kib(category_memory_before, read_process_memory_stats());
                        let category_trimmed = trim_allocator_memory();
                        let category_rss_delta_kib =
                            rss_delta_kib(category_memory_before, read_process_memory_stats());
                        let cycle_elapsed_ms =
                            cycle_started.elapsed().as_millis().min(u64::MAX as u128) as u64;
                        let cycle_rss_delta_kib =
                            rss_delta_kib(cycle_memory_before, read_process_memory_stats());
                        store_runtime_progress(
                            success_count,
                            failure_count,
                            page_count,
                            raw_item_count,
                            total_count,
                            candidate_event_count,
                            cycle_elapsed_ms,
                            cycle_rss_delta_kib,
                        );
                        crate::logging::info_file_async(format!(
                            "Yahoo category diagnostics | {} {}({}) failed=true total={} pages=0 raw_items=0 changed_events=0 rss_delta={}KiB rss_delta_before_trim={}KiB trim={} elapsed={}ms",
                            category.exchange.label(),
                            category.name,
                            category.sector_id,
                            total_count,
                            category_rss_delta_kib,
                            category_rss_delta_before_trim_kib,
                            category_trimmed,
                            started_at.elapsed().as_millis().min(u64::MAX as u128) as u64,
                        ));
                        // 類股失敗時只記錄錯誤，不中止整輪任務，
                        // 避免單一 sector 出問題就拖垮整個 Yahoo 報價快取。
                        crate::logging::error_file_async(format!(
                            "Yahoo 類股快取更新失敗: {} {}({}) {:?}",
                            category.exchange.label(),
                            category.name,
                            category.sector_id,
                            why
                        ));
                    }
                }

                // 類股處理完後再檢查一次停止旗標，
                // 讓 stop 能在兩個類股之間盡快生效。
                if !IS_CACHING.load(Ordering::SeqCst) {
                    break;
                }

                // 類股與類股之間固定 sleep，目的是降低連續高頻請求被 Yahoo 視為異常流量的機率。
                sleep(CATEGORY_REQUEST_INTERVAL).await;
            }

            // 如果是在整輪尾端才收到 stop，就不要再進入 cooldown。
            if !IS_CACHING.load(Ordering::SeqCst) {
                break;
            }

            // 讀共享快取目前總筆數，只拿來做可觀測性 log，不參與任何商業判斷。
            let total_count = SHARE
                .stock_snapshots
                .read()
                .map(|cache| cache.len())
                .unwrap_or_default();

            // 若整輪結束後共享快取仍然是空的，代表本輪 Yahoo 採集沒有成功落地任何資料，
            // 這是一種需要人回頭檢查程式或來源格式的明確異常。
            if total_count == 0 {
                crate::logging::error_file_async(format!(
                    "Yahoo 類股快取輪詢完成但沒有任何資料落地: success_count={} failure_count={} 耗時 {:?}",
                    success_count,
                    failure_count,
                    cycle_started.elapsed()
                ));
            }

            crate::logging::debug_file_async(format!(
                "Yahoo 類股快取輪詢完成，共 {} 檔股票，成功類股 {}，失敗類股 {}，candidate_events={}，耗時 {:?}",
                total_count,
                success_count,
                failure_count,
                candidate_event_count,
                cycle_started.elapsed()
            ));

            let cycle_elapsed_ms = cycle_started.elapsed().as_millis().min(u64::MAX as u128) as u64;
            let cycle_rss_delta_before_trim_kib =
                rss_delta_kib(cycle_memory_before, read_process_memory_stats());
            let allocator_trimmed = trim_allocator_memory();
            let cycle_rss_delta_kib =
                rss_delta_kib(cycle_memory_before, read_process_memory_stats());
            store_runtime_progress(
                success_count,
                failure_count,
                page_count,
                raw_item_count,
                total_count,
                candidate_event_count,
                cycle_elapsed_ms,
                cycle_rss_delta_kib,
            );
            COMPLETED_CYCLES.fetch_add(1, Ordering::SeqCst);
            crate::logging::info_file_async(format!(
                "Yahoo cycle diagnostics | total={} success={} failure={} pages={} raw_items={} candidate_events={} rss_delta={}KiB rss_delta_before_trim={}KiB trim={} elapsed={}ms",
                total_count,
                success_count,
                failure_count,
                page_count,
                raw_item_count,
                candidate_event_count,
                cycle_rss_delta_kib,
                cycle_rss_delta_before_trim_kib,
                allocator_trimmed,
                cycle_elapsed_ms,
            ));

            // 一輪全部類股跑完後稍作休息，避免無間斷全市場輪詢造成壓力過大。
            sleep(CYCLE_COOLDOWN).await;
        }

        // 跳出 while 代表旗標已關閉，這裡補一筆停止 log 方便對照啟停時間。
        IS_CACHING.store(false, Ordering::SeqCst);
        let active_tasks = decrement_active_tasks(&ACTIVE_TASKS);
        crate::logging::info_file_async(format!(
            "Yahoo 類股快取任務已停止 generation={} active_tasks={}",
            generation, active_tasks
        ));
    });

    if let Ok(mut task) = CACHING_TASK.lock() {
        *task = Some(handle);
    } else {
        crate::logging::error_file_async("Failed to store Yahoo caching task handle".to_string());
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
            crate::logging::error_file_async(format!(
                "Failed to lock Yahoo caching task handle because {:?}",
                why
            ));
            None
        }
    };

    if let Some(handle) = handle {
        if let Err(why) = handle.await {
            crate::logging::error_file_async(format!("Yahoo 類股快取任務停止等待失敗: {:?}", why));
        }
    }

    // 然後主動清空快取，避免收盤或停任務後外部仍讀到過期盤中報價。
    SHARE.clear_stock_snapshots();
}

/// 取得 Yahoo 類股背景任務目前的執行狀態。
pub(crate) fn diagnostics_snapshot() -> TaskRuntimeStatus {
    TaskRuntimeStatus::new(
        IS_CACHING.load(Ordering::SeqCst),
        ACTIVE_TASKS.load(Ordering::SeqCst),
        LAST_TASK_GENERATION.load(Ordering::SeqCst),
    )
}

/// 取得 Yahoo 最近一輪輪詢的執行摘要。
pub(crate) fn runtime_diagnostics_snapshot() -> YahooRuntimeDiagnostics {
    YahooRuntimeDiagnostics {
        status: diagnostics_snapshot(),
        last_success_count: LAST_SUCCESS_COUNT.load(Ordering::SeqCst),
        last_failure_count: LAST_FAILURE_COUNT.load(Ordering::SeqCst),
        last_page_count: LAST_PAGE_COUNT.load(Ordering::SeqCst),
        last_raw_item_count: LAST_RAW_ITEM_COUNT.load(Ordering::SeqCst),
        last_snapshot_count: LAST_SNAPSHOT_COUNT.load(Ordering::SeqCst),
        last_candidate_event_count: LAST_CANDIDATE_EVENT_COUNT.load(Ordering::SeqCst),
        last_elapsed_ms: LAST_ELAPSED_MS.load(Ordering::SeqCst),
        last_rss_delta_kib: LAST_RSS_DELTA_KIB.load(Ordering::SeqCst),
        completed_cycles: COMPLETED_CYCLES.load(Ordering::SeqCst),
    }
}

/// 單一類股快取套用後的摘要。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ApplyCategoryResult {
    total_snapshot_count: usize,
    changed_event_count: usize,
}

/// 將單一類股的最新快照套用到共用快取。
///
/// 這個步驟會：
/// - 移除該類股上一輪存在、這一輪已消失的股票。
/// - 寫入該類股本輪抓到的最新快照。
/// - 回傳更新後整體快取的股票數量。
fn apply_category_snapshots(
    category: &YahooClassCategory,
    category_snapshots: HashMap<String, RealtimeSnapshot>,
    category_symbols: &mut HashMap<String, HashSet<String>>,
) -> ApplyCategoryResult {
    // 先算出這個類股本輪的內部鍵值，讓同一個 sector 的 symbol 集可以被覆蓋更新。
    let category_key = class_quote::category_key(category);
    // 把本輪所有 symbol 收成集合，後面可以和上一輪做集合差異比對。
    let new_symbols: HashSet<String> = category_snapshots.keys().cloned().collect();
    // `insert` 會回傳舊集合；這正好拿來得知上一輪這個類股有哪些股票。
    let previous_symbols = category_symbols
        .insert(category_key, new_symbols)
        .unwrap_or_default();

    match SHARE.stock_snapshots.write() {
        Ok(mut cache) => {
            let mut changed_event_count = 0usize;
            // 先刪除這個類股上一輪有、這一輪沒有的股票，
            // 避免共享快取殘留已不在該類股結果中的舊資料。
            for symbol in previous_symbols {
                if !category_snapshots.contains_key(&symbol) {
                    cache.remove(&symbol);
                }
            }

            // 再把本輪抓到的快照逐筆寫回共享快取。
            // 若 symbol 已存在，就用最新 snapshot 覆蓋。
            for (symbol, snapshot) in category_snapshots {
                let price = snapshot.price;
                let has_changed = snapshot.price != Decimal::ZERO
                    && cache
                        .get(&symbol)
                        .is_none_or(|old_snapshot| old_snapshot.price != snapshot.price);
                let publish_symbol = has_changed.then(|| symbol.clone());
                cache.insert(symbol, snapshot);

                if let Some(symbol) = publish_symbol {
                    changed_event_count += 1;
                    trace_price_tasks::publish_price_update(symbol, price);
                }
            }

            // 回傳整體共享快取筆數，讓呼叫端能寫 log 觀察目前快取規模。
            ApplyCategoryResult {
                total_snapshot_count: cache.len(),
                changed_event_count,
            }
        }
        Err(why) => {
            // 寫鎖失敗時記錄錯誤，並回傳 0 讓 log 明顯顯示這輪更新沒有成功落地。
            crate::logging::error_file_async(format!(
                "Failed to update Yahoo 類股快取 because {:?}",
                why
            ));
            ApplyCategoryResult::default()
        }
    }
}

fn rss_delta_kib(before: Option<ProcessMemoryStats>, after: Option<ProcessMemoryStats>) -> i64 {
    match (before, after) {
        (Some(before), Some(after)) => after.vm_rss_kib as i64 - before.vm_rss_kib as i64,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::Mutex,
        time::{Duration, Instant},
    };

    use once_cell::sync::Lazy;
    use rust_decimal_macros::dec;
    use tokio::time::sleep;

    use super::*;

    static TEST_STATE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    /// 驗證快取全量更新前，只會針對價格實際異動的股票產生事件。
    #[test]
    fn test_apply_category_snapshots_counts_changed_prices() {
        let _lock = TEST_STATE_LOCK.lock().unwrap();
        SHARE.clear_stock_snapshots();

        let category = YahooClassCategory::enabled(
            crate::crawler::yahoo::YahooClassExchange::Listed,
            40,
            "半導體",
        );
        let mut category_symbols = HashMap::new();

        let mut existing = HashMap::new();
        existing.insert(
            "2330".to_string(),
            RealtimeSnapshot::new("2330".to_string(), dec!(998)),
        );
        SHARE.set_stock_snapshots(existing);

        let mut new_data = HashMap::new();
        new_data.insert(
            "2330".to_string(),
            RealtimeSnapshot::new("2330".to_string(), dec!(1000)),
        );
        new_data.insert(
            "2317".to_string(),
            RealtimeSnapshot::new("2317".to_string(), dec!(180)),
        );
        new_data.insert(
            "2454".to_string(),
            RealtimeSnapshot::new("2454".to_string(), Decimal::ZERO),
        );

        let result = apply_category_snapshots(&category, new_data, &mut category_symbols);

        assert_eq!(result.changed_event_count, 2);

        SHARE.clear_stock_snapshots();
    }

    /// 驗證同一個類股重新更新時，已不存在的股票會從共用快取中移除。
    #[test]
    fn test_apply_category_snapshots_replaces_removed_symbols_in_same_category() {
        let _lock = TEST_STATE_LOCK.lock().unwrap();
        SHARE.clear_stock_snapshots();

        let category = YahooClassCategory::enabled(
            crate::crawler::yahoo::YahooClassExchange::Listed,
            40,
            "半導體",
        );
        let mut category_symbols = HashMap::new();

        let mut first = HashMap::new();
        first.insert(
            "2330".to_string(),
            RealtimeSnapshot::new("2330".to_string(), dec!(998)),
        );
        first.insert(
            "2303".to_string(),
            RealtimeSnapshot::new("2303".to_string(), dec!(45)),
        );
        apply_category_snapshots(&category, first, &mut category_symbols);

        let mut second = HashMap::new();
        second.insert(
            "2330".to_string(),
            RealtimeSnapshot::new("2330".to_string(), dec!(999)),
        );
        apply_category_snapshots(&category, second, &mut category_symbols);

        let cache = SHARE.stock_snapshots.read().unwrap();
        assert!(cache.contains_key("2330"));
        assert!(!cache.contains_key("2303"));
    }

    /// Live 測試：驗證啟動背景任務後快取會落地，停止後會被清空。
    #[tokio::test]
    #[ignore]
    async fn test_start_and_stop_caching_task_integration() {
        let _lock = TEST_STATE_LOCK.lock().unwrap();
        const CACHE_WARMUP_TIMEOUT: Duration = Duration::from_secs(30);
        const CACHE_WARMUP_POLL_INTERVAL: Duration = Duration::from_millis(500);

        stop_caching_task().await;
        start_caching_task();

        let started_at = Instant::now();
        loop {
            let is_ready = SHARE
                .stock_snapshots
                .read()
                .map(|cache| !cache.is_empty())
                .unwrap_or(false);

            if is_ready {
                break;
            }

            assert!(
                started_at.elapsed() < CACHE_WARMUP_TIMEOUT,
                "Yahoo 類股快取在 {:?} 內未成功落地",
                CACHE_WARMUP_TIMEOUT
            );

            sleep(CACHE_WARMUP_POLL_INTERVAL).await;
        }

        let snapshot_count = SHARE
            .stock_snapshots
            .read()
            .map(|cache| cache.len())
            .unwrap_or_default();
        assert!(snapshot_count > 0);

        stop_caching_task().await;
        sleep(Duration::from_millis(100)).await;

        let is_empty = SHARE
            .stock_snapshots
            .read()
            .map(|cache| cache.is_empty())
            .unwrap_or(false);
        assert!(is_empty, "Yahoo 類股快取停止後應為空");
    }
}
