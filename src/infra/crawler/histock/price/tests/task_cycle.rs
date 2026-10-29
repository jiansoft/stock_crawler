//! HiStock 背景採集任務：快取寫入、診斷計數、失敗分級與每輪結果處理的測試。

use super::*;

/// 驗證全量快取更新前，只會針對價格實際異動的股票產生價格事件。
#[test]
fn test_collect_changed_price_updates() {
    let _guard = TEST_STATE_LOCK.blocking_lock();
    SHARE.clear_stock_snapshots();

    let mut existing_snapshot = RealtimeSnapshot::new("2330".to_string(), dec!(998));
    existing_snapshot.name = "台積電".to_string();
    let mut cache = HashMap::new();
    cache.insert("2330".to_string(), existing_snapshot);
    SHARE.set_stock_snapshots(cache);

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

    let mut updates = collect_changed_price_updates(&new_data);
    updates.sort_by(|left, right| left.0.cmp(&right.0));

    assert_eq!(
        updates,
        vec![
            ("2317".to_string(), dec!(180)),
            ("2330".to_string(), dec!(1000)),
        ]
    );

    SHARE.clear_stock_snapshots();
}

#[test]
fn diagnostics_snapshot_reflects_idle_state_and_runtime_counters() {
    // 與 record_cycle 測試共用診斷計數器，需序列化。
    let _guard = TEST_STATE_LOCK.blocking_lock();
    IS_CACHING.store(false, Ordering::SeqCst);
    ACTIVE_TASKS.store(0, Ordering::SeqCst);
    LAST_BODY_BYTES.store(1024, Ordering::SeqCst);
    LAST_ROW_COUNT.store(30, Ordering::SeqCst);
    LAST_SNAPSHOT_COUNT.store(28, Ordering::SeqCst);
    LAST_CHANGED_EVENT_COUNT.store(4, Ordering::SeqCst);
    LAST_ELAPSED_MS.store(250, Ordering::SeqCst);
    LAST_RSS_DELTA_KIB.store(-12, Ordering::SeqCst);
    COMPLETED_CYCLES.store(3, Ordering::SeqCst);

    let diagnostics = runtime_diagnostics_snapshot();

    assert!(!diagnostics.status.enabled);
    assert_eq!(diagnostics.status.active_tasks, 0);
    assert_eq!(diagnostics.last_body_bytes, 1024);
    assert_eq!(diagnostics.last_row_count, 30);
    assert_eq!(diagnostics.last_snapshot_count, 28);
    assert_eq!(diagnostics.last_changed_event_count, 4);
    assert_eq!(diagnostics.last_elapsed_ms, 250);
    assert_eq!(diagnostics.last_rss_delta_kib, -12);
    assert_eq!(diagnostics.completed_cycles, 3);
}

#[tokio::test]
async fn test_cache_mechanism() {
    let _guard = TEST_STATE_LOCK.lock().await;
    stop_caching_task().await;
    {
        assert!(
            SHARE.stock_snapshots.read().unwrap().is_empty(),
            "Cache should be empty after stop_caching_task"
        );
    }

    let mock_symbol = "MOCK99";
    let mut mock_snapshot = RealtimeSnapshot::new(mock_symbol.to_string(), dec!(100.5));
    mock_snapshot.name = "測試股".to_string();
    mock_snapshot.change = dec!(1.5);
    mock_snapshot.change_range = dec!(1.51);
    mock_snapshot.open = dec!(99.0);
    mock_snapshot.high = dec!(101.0);
    mock_snapshot.low = dec!(98.5);
    mock_snapshot.last_close = dec!(99.0);
    mock_snapshot.volume = dec!(500);

    {
        let mut cache = SHARE.stock_snapshots.write().unwrap();
        cache.insert(mock_symbol.to_string(), mock_snapshot.clone());
    }

    let result = get_snapshot(mock_symbol).await.unwrap();
    assert_eq!(result, mock_snapshot);

    stop_caching_task().await;
    {
        assert!(SHARE.stock_snapshots.read().unwrap().is_empty());
    }
}

/// RSS 差值：前後都讀得到才相減（可為負），任一側讀不到就回 0。
#[test]
fn rss_delta_kib_needs_both_samples() {
    let sample = |vm_rss_kib| ProcessMemoryStats {
        vm_rss_kib,
        vm_size_kib: 0,
    };

    assert_eq!(rss_delta_kib(Some(sample(1_000)), Some(sample(1_250))), 250);
    assert_eq!(
        rss_delta_kib(Some(sample(1_250)), Some(sample(1_000))),
        -250
    );
    assert_eq!(rss_delta_kib(None, Some(sample(1_000))), 0);
    assert_eq!(rss_delta_kib(Some(sample(1_000)), None), 0);
}

/// 快取命中時，`StockInfo` 兩個介面直接從快取取值，不會觸發全量重抓。
#[tokio::test]
async fn stock_info_reads_from_cache_when_hit() {
    let _guard = TEST_STATE_LOCK.lock().await;
    stop_caching_task().await;

    let mut snapshot = consistent_snapshot("79974");
    snapshot.change_range = dec!(4.11);
    {
        let mut cache = SHARE.stock_snapshots.write().unwrap();
        cache.insert("79974".to_string(), snapshot);
    }

    let price = HiStock::get_stock_price("79974").await.unwrap();
    let quotes = HiStock::get_stock_quotes("79974").await.unwrap();

    assert_eq!(price, dec!(78.5));
    assert_eq!(quotes.stock_symbol, "79974");
    assert_eq!(quotes.price, 78.5);
    assert_eq!(quotes.change, 3.1);
    assert_eq!(quotes.change_range, 4.11);

    stop_caching_task().await;
}

/// 一輪成功的抓取會覆蓋快取、更新診斷計數器並累加完成輪數。
#[tokio::test]
async fn record_cycle_updates_cache_and_counters() {
    let _guard = TEST_STATE_LOCK.lock().await;
    stop_caching_task().await;
    let cycles_before = COMPLETED_CYCLES.load(Ordering::SeqCst);
    let mut result = fetch_result_with(vec![
        consistent_snapshot("79977"),
        consistent_snapshot("79978"),
    ]);
    result.body_bytes = 2048;

    // 成功一輪就把連續失敗歸零。
    CONSECUTIVE_FAILURES.store(3, Ordering::SeqCst);
    record_cycle(result, std::time::Instant::now(), None);
    assert_eq!(CONSECUTIVE_FAILURES.load(Ordering::SeqCst), 0);

    assert_eq!(
        SHARE.get_stock_snapshot("79977").map(|s| s.price),
        Some(dec!(78.5))
    );
    assert_eq!(LAST_BODY_BYTES.load(Ordering::SeqCst), 2048);
    assert_eq!(LAST_ROW_COUNT.load(Ordering::SeqCst), 2);
    assert_eq!(LAST_SNAPSHOT_COUNT.load(Ordering::SeqCst), 2);
    assert_eq!(LAST_CHANGED_EVENT_COUNT.load(Ordering::SeqCst), 2);
    assert_eq!(LAST_RSS_DELTA_KIB.load(Ordering::SeqCst), 0);
    assert_eq!(COMPLETED_CYCLES.load(Ordering::SeqCst), cycles_before + 1);

    stop_caching_task().await;
}

/// 連續失敗未滿 12 輪記 warn，第 12 輪與之後每 120 輪記 error，其餘 debug。
#[test]
fn failure_level_escalates_after_consecutive_failures() {
    use tracing::Level;
    assert_eq!(failure_level(3), Level::WARN);
    for failures in [12, 132, 252] {
        assert_eq!(failure_level(failures), Level::ERROR, "{failures}");
    }
    for failures in [1, 2, 11, 13, 131, 133] {
        assert_eq!(failure_level(failures), Level::DEBUG, "{failures}");
    }
}

/// 整批錯亂在當天、當下時段只記第一筆（之後記 debug）；其他錯誤依連續失敗輪數記錄，都不會中斷迴圈。
#[test]
fn report_cycle_error_marks_dirty_batch_as_seen_today() {
    let dirty = anyhow::Error::from(DirtyBatch {
        inconsistent: 30,
        change_mismatch: 30,
        total: 100,
        ..DirtyBatch::default()
    });

    let _guard = TEST_STATE_LOCK.blocking_lock();
    CONSECUTIVE_FAILURES.store(0, Ordering::SeqCst);
    report_cycle_error(&dirty);
    report_cycle_error(&dirty);
    // 整批錯亂不算連續失敗；其他錯誤每輪加一。
    assert_eq!(CONSECUTIVE_FAILURES.load(Ordering::SeqCst), 0);
    report_cycle_error(&anyhow::anyhow!("連線逾時"));
    report_cycle_error(&anyhow::anyhow!("連線重置"));
    assert_eq!(CONSECUTIVE_FAILURES.load(Ordering::SeqCst), 2);
    for _ in 0..20 {
        report_cycle_error(&anyhow::anyhow!("持續逾時"));
    }
    assert_eq!(CONSECUTIVE_FAILURES.load(Ordering::SeqCst), 22);
    CONSECUTIVE_FAILURES.store(0, Ordering::SeqCst);

    // 當下時段（開盤或盤中）的那一筆已經記過，再來只記 debug。
    assert_eq!(
        dirty_batch_level(chrono::Local::now().naive_local()),
        tracing::Level::DEBUG
    );
}

/// 09:15 前的整批錯亂記 info、之後記 warn，兩段各自每天只記第一筆，其餘 debug。
#[test]
fn dirty_batch_level_is_info_during_the_opening_window() {
    use tracing::Level;
    let _guard = TEST_STATE_LOCK.blocking_lock();
    let day = chrono::NaiveDate::from_ymd_opt(2020, 3, 2).unwrap();
    let at = |h, m| day.and_hms_opt(h, m, 0).unwrap();
    assert_eq!(dirty_batch_level(at(9, 2)), Level::INFO);
    assert_eq!(dirty_batch_level(at(9, 10)), Level::DEBUG);
    assert_eq!(dirty_batch_level(at(9, 15)), Level::WARN);
    assert_eq!(dirty_batch_level(at(11, 0)), Level::DEBUG);
    let next = day.succ_opt().unwrap().and_hms_opt(10, 0, 0).unwrap();
    assert_eq!(dirty_batch_level(next), Level::WARN);
}

/// 全量重抓後取出指定股票，並以新資料覆蓋快取。
#[tokio::test]
async fn store_and_pick_returns_symbol_and_refreshes_cache() {
    let _guard = TEST_STATE_LOCK.lock().await;
    stop_caching_task().await;

    let snapshot = store_and_pick(
        "79977",
        fetch_result_with(vec![
            consistent_snapshot("79977"),
            consistent_snapshot("79978"),
        ]),
    )
    .unwrap();

    assert_eq!(snapshot.symbol, "79977");
    assert_eq!(
        SHARE.get_stock_snapshot("79978").map(|s| s.price),
        Some(dec!(78.5))
    );

    stop_caching_task().await;
}

/// 目標股票不在排行頁時回錯，且不覆蓋快取。
#[tokio::test]
async fn store_and_pick_rejects_missing_symbol_without_touching_cache() {
    let _guard = TEST_STATE_LOCK.lock().await;
    stop_caching_task().await;

    let result = store_and_pick(
        "79979",
        fetch_result_with(vec![consistent_snapshot("79977")]),
    );

    assert!(result.is_err());
    assert!(SHARE.get_stock_snapshot("79977").is_none());
}
