//! Yahoo 類股快取背景任務的測試：輪詢週期、WAF 冷卻、停止旗標與快取清空。

use std::time::{Duration, Instant};

use tokio::time::sleep;

use super::*;

/// 沒有背景任務時停止也要清空共用快取，避免收盤後仍讀到盤中報價。
#[tokio::test]
async fn stop_caching_task_clears_snapshots_without_a_running_task() {
    let _lock = TEST_STATE_LOCK.lock().await;
    let mut snapshots = HashMap::new();
    snapshots.insert(
        "2330".to_string(),
        crate::infra::cache::RealtimeSnapshot::new(
            "2330".to_string(),
            rust_decimal::Decimal::ONE_HUNDRED,
        ),
    );
    SHARE.set_stock_snapshots(snapshots);

    stop_caching_task().await;

    assert!(!IS_CACHING.load(Ordering::SeqCst));
    assert!(SHARE.stock_snapshots.read().unwrap().is_empty());
}

/// 停止旗標關閉時，長時間冷卻立刻返回，不會拖住服務停止。
#[tokio::test]
async fn sleep_while_caching_returns_at_once_when_stopped() {
    let _lock = TEST_STATE_LOCK.lock().await;
    IS_CACHING.store(false, Ordering::SeqCst);
    let started = Instant::now();
    sleep_while_caching(DENIED_COOLDOWN).await;
    assert!(started.elapsed() < Duration::from_secs(1));
}

use std::sync::Mutex;

use anyhow::anyhow;
use rust_decimal::Decimal;

use crate::infra::{
    cache::RealtimeSnapshot,
    crawler::yahoo::{YahooClassExchange, price::class_quote::CategoryFetchDiagnostics},
};

fn category(sector_id: u32) -> YahooClassCategory {
    YahooClassCategory::enabled(YahooClassExchange::Listed, sector_id, "測試類股")
}

/// 一頁、指定股票與價格的類股抓取結果。
fn fetched(symbols: &[(&str, i64)]) -> CategoryFetchResult {
    CategoryFetchResult {
        snapshots: symbols
            .iter()
            .map(|(symbol, price)| {
                (
                    (*symbol).to_string(),
                    RealtimeSnapshot::new((*symbol).to_string(), Decimal::from(*price)),
                )
            })
            .collect(),
        diagnostics: CategoryFetchDiagnostics {
            page_count: 1,
            raw_item_count: symbols.len(),
            snapshot_count: symbols.len(),
        },
    }
}

/// 開始輪詢前的共用狀態：旗標開啟、快取清空（避免真實昨收價影響價格驗證）。
fn reset_shared_state() {
    IS_CACHING.store(true, Ordering::SeqCst);
    SHARE.clear_stock_snapshots();
    SHARE.clear_last_trading_day_quotes();
}

fn cached(symbol: &str) -> bool {
    SHARE.stock_snapshots.read().unwrap().contains_key(symbol)
}

/// 全部類股成功：寫回共用快取、累計分頁與異動數，診斷數據與完成輪數跟著更新。
#[tokio::test(start_paused = true)]
async fn run_cycle_applies_every_category_and_records_progress() {
    let _lock = TEST_STATE_LOCK.lock().await;
    reset_shared_state();
    let (first, second) = (category(9001), category(9002));
    let categories = [&first, &second];
    let mut category_symbols = HashMap::new();

    let stats = run_cycle(
        &categories,
        &mut category_symbols,
        Instant::now(),
        None,
        |category: &YahooClassCategory| {
            let result = if category.sector_id == 9001 {
                fetched(&[("79979Y1", 100), ("79979Y2", 50)])
            } else {
                fetched(&[("79979Y3", 20)])
            };
            async move { Ok(result) }
        },
    )
    .await;

    assert_eq!(
        stats,
        CycleStats {
            success: 2,
            failure: 0,
            pages: 2,
            raw_items: 3,
            candidate_events: 3,
            denied: false,
        }
    );
    assert!(cached("79979Y1") && cached("79979Y3"));
    let diagnostics = runtime_diagnostics_snapshot();
    assert_eq!(diagnostics.last_success_count, 2);
    assert_eq!(diagnostics.last_snapshot_count, 3);

    let completed = COMPLETED_CYCLES.load(Ordering::SeqCst);
    assert_eq!(finish_cycle(&stats, Instant::now(), None), CYCLE_COOLDOWN);
    assert_eq!(COMPLETED_CYCLES.load(Ordering::SeqCst), completed + 1);

    IS_CACHING.store(false, Ordering::SeqCst);
    SHARE.clear_stock_snapshots();
}

/// 暫時性 5xx 只記失敗、繼續下一個類股；遭遇 WAF 阻擋就中止剩餘類股並改用長冷卻。
#[tokio::test(start_paused = true)]
async fn run_cycle_stops_at_waf_denial_and_cools_down_longer() {
    let _lock = TEST_STATE_LOCK.lock().await;
    reset_shared_state();
    let (transient, denied, skipped) = (category(9101), category(9102), category(9103));
    let categories = [&transient, &denied, &skipped];
    let calls = Mutex::new(Vec::new());

    let stats = run_cycle(
        &categories,
        &mut HashMap::new(),
        Instant::now(),
        None,
        |category: &YahooClassCategory| {
            calls.lock().unwrap().push(category.sector_id);
            let result = match category.sector_id {
                9101 => Err(anyhow!("Yahoo class quote request failed with status 502")),
                9102 => Err(anyhow!("Request denied")),
                _ => Ok(fetched(&[("79979Y9", 10)])),
            };
            async move { result }
        },
    )
    .await;

    assert_eq!(
        *calls.lock().unwrap(),
        vec![9101, 9102],
        "阻擋後不應再抓第三個類股"
    );
    assert_eq!((stats.success, stats.failure, stats.denied), (0, 2, true));
    assert!(!cached("79979Y9"));
    assert_eq!(finish_cycle(&stats, Instant::now(), None), DENIED_COOLDOWN);

    IS_CACHING.store(false, Ordering::SeqCst);
}

/// 停止旗標關閉後不再抓取；請求進行中被停止時，回來的資料也不寫回快取。
#[tokio::test(start_paused = true)]
async fn run_cycle_honours_the_stop_flag() {
    let _lock = TEST_STATE_LOCK.lock().await;
    reset_shared_state();
    let only = category(9201);
    let categories = [&only];

    let stats = run_cycle(
        &categories,
        &mut HashMap::new(),
        Instant::now(),
        None,
        |_: &YahooClassCategory| {
            // 模擬 HTTP 請求進行中收到 stop。
            IS_CACHING.store(false, Ordering::SeqCst);
            async { Ok(fetched(&[("79979Y5", 10)])) }
        },
    )
    .await;
    assert_eq!(stats, CycleStats::default());
    assert!(!cached("79979Y5"));

    let calls = Mutex::new(0);
    let stats = run_cycle(
        &categories,
        &mut HashMap::new(),
        Instant::now(),
        None,
        |_: &YahooClassCategory| {
            *calls.lock().unwrap() += 1;
            async { Ok(fetched(&[])) }
        },
    )
    .await;
    assert_eq!((stats, *calls.lock().unwrap()), (CycleStats::default(), 0));
}

/// Live 測試：驗證啟動背景任務後快取會落地，停止後會被清空。
#[tokio::test]
#[ignore]
async fn test_start_and_stop_caching_task_integration() {
    let _lock = TEST_STATE_LOCK.lock().await;
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
