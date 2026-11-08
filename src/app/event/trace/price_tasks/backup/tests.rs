//! 被追蹤股票的備援採集任務的測試。

use rust_decimal_macros::dec;

use super::*;
use crate::infra::cache::RealtimeSnapshot;

/// 不存在於股票主檔的測試代號，避免與真實報價或其他測試互相干擾。
const SYMBOL_NEW: &str = "79961";
const SYMBOL_SAME: &str = "79962";
const SYMBOL_ABNORMAL: &str = "79963";
const SYMBOL_ZERO: &str = "79964";

fn fetched(price: Decimal, site_name: &'static str) -> Result<FetchedStockPrice> {
    Ok(FetchedStockPrice { price, site_name })
}

fn put_snapshot(symbol: &str, price: Decimal, last_close: Decimal, source_site: &str) {
    let mut snapshot = RealtimeSnapshot::new(symbol.to_string(), price);
    snapshot.last_close = last_close;
    snapshot.source_site = source_site.to_string();
    SHARE
        .stock_snapshots
        .write()
        .expect("snapshot lock")
        .insert(symbol.to_string(), snapshot);
}

fn remove_snapshot(symbol: &str) {
    SHARE
        .stock_snapshots
        .write()
        .expect("snapshot lock")
        .remove(symbol);
}

/// 快取沒有這檔時寫入新價格與來源，並視為價格異動。
#[test]
fn new_price_is_cached_and_reported_as_changed() {
    let _lock = crate::infra::cache::REALTIME_TEST_LOCK.blocking_lock();
    remove_snapshot(SYMBOL_NEW);

    assert!(apply_backup_price(
        SYMBOL_NEW.to_string(),
        fetched(dec!(101.5), "CnYes")
    ));
    let snapshot = SHARE.get_stock_snapshot(SYMBOL_NEW).expect("應已寫入快取");
    assert_eq!(snapshot.price, dec!(101.5));
    assert_eq!(snapshot.source_site, "CnYes");

    remove_snapshot(SYMBOL_NEW);
}

/// 價格與來源都沒變不更新；只換來源時更新來源但不算價格異動。
#[test]
fn unchanged_price_is_not_reported() {
    let _lock = crate::infra::cache::REALTIME_TEST_LOCK.blocking_lock();
    put_snapshot(SYMBOL_SAME, dec!(50), dec!(50), "Fugle");

    assert!(!apply_backup_price(
        SYMBOL_SAME.to_string(),
        fetched(dec!(50), "Fugle")
    ));
    assert!(!apply_backup_price(
        SYMBOL_SAME.to_string(),
        fetched(dec!(50), "PcHome")
    ));
    let snapshot = SHARE.get_stock_snapshot(SYMBOL_SAME).expect("快取應存在");
    assert_eq!(snapshot.source_site, "PcHome");
    assert_eq!(snapshot.price, dec!(50));

    remove_snapshot(SYMBOL_SAME);
}

/// 超出漲跌幅的價格（HiStock 偶發錯價那類）必須被過濾，快取維持原值。
#[test]
fn abnormal_price_is_filtered() {
    let _lock = crate::infra::cache::REALTIME_TEST_LOCK.blocking_lock();
    put_snapshot(SYMBOL_ABNORMAL, dec!(54.7), dec!(54.7), "Yahoo");

    assert!(!apply_backup_price(
        SYMBOL_ABNORMAL.to_string(),
        fetched(dec!(141.5), "CMoney")
    ));
    let snapshot = SHARE
        .get_stock_snapshot(SYMBOL_ABNORMAL)
        .expect("快取應存在");
    assert_eq!(snapshot.price, dec!(54.7));
    assert_eq!(snapshot.source_site, "Yahoo");

    remove_snapshot(SYMBOL_ABNORMAL);
}

/// 0 元（尚未成交）與抓取失敗都不寫入快取。
#[test]
fn zero_price_and_fetch_errors_are_ignored() {
    let _lock = crate::infra::cache::REALTIME_TEST_LOCK.blocking_lock();
    remove_snapshot(SYMBOL_ZERO);

    assert!(!apply_backup_price(
        SYMBOL_ZERO.to_string(),
        fetched(Decimal::ZERO, "CnYes")
    ));
    assert!(!apply_backup_price(
        SYMBOL_ZERO.to_string(),
        Err(anyhow::anyhow!("all backup sites failed"))
    ));
    assert!(SHARE.get_stock_snapshot(SYMBOL_ZERO).is_none());
}

/// 沒有被追蹤的股票時不發任何請求，直接成功。
#[tokio::test]
async fn refresh_without_traced_symbols_is_a_noop() {
    let _lock = crate::infra::cache::REALTIME_TEST_LOCK.lock().await;
    stock_price::clear_trace_targets_cache();
    refresh_traced_stock_snapshot_cache()
        .await
        .expect("空清單應直接成功");
}

/// 背景任務可以啟動與停止；非交易時段會自行結束，交易時段則由停止旗標結束。
#[tokio::test(start_paused = true)]
async fn backup_task_starts_and_stops() {
    let _lock = crate::infra::cache::REALTIME_TEST_LOCK.lock().await;
    stock_price::clear_trace_targets_cache();
    start_traced_stock_backup_caching_task();

    stop_traced_stock_backup_caching_task();
    for _ in 0..100 {
        super::super::TRACE_TASK_STOP_NOTIFY.notify_waiters();
        tokio::time::sleep(Duration::from_millis(50)).await;
        if BACKUP_ACTIVE_TASKS.load(Ordering::SeqCst) == 0
            && !IS_BACKUP_CACHING.load(Ordering::SeqCst)
        {
            return;
        }
    }
    panic!("備援採集任務未在期限內停止");
}
