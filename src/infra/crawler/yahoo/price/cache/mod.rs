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

use tokio::time::sleep;

use crate::{
    core::util::{atomic::decrement_atomic_usize, diagnostics::read_process_memory_stats},
    infra::{cache::SHARE, crawler::yahoo::YahooClassCategory},
};

use super::class_quote::{self, CategoryFetchResult};

/// 類股快照寫回共用快取。
mod apply;
/// 一輪類股輪詢與結束摘要。
mod cycle;
/// 類股 API 錯誤分類。
mod errors;
/// 生命週期旗標與診斷數據。
mod state;

use cycle::{finish_cycle, run_cycle};
use state::{ACTIVE_TASKS, CACHING_TASK, IS_CACHING, LAST_TASK_GENERATION};
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

/// 背景任務本體：停止旗標關閉前反覆輪詢全部類股，每輪結束依結果冷卻。
///
/// 抓取函式由參數傳入，測試可以不連外直接跑完整的輪詢與停止流程。
async fn caching_loop<'a, F, Fut>(generation: u64, categories: &[&'a YahooClassCategory], fetch: F)
where
    F: Fn(&'a YahooClassCategory) -> Fut,
    Fut: Future<Output = anyhow::Result<CategoryFetchResult>>,
{
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
            categories,
            &mut category_symbols,
            cycle_started,
            cycle_memory_before,
            &fetch,
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
        caching_loop(
            generation,
            &categories,
            class_quote::fetch_category_snapshots,
        )
        .await;
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
