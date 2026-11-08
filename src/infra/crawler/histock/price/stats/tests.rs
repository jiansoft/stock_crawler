//! HiStock 每日背景採集統計；逐列異常留待收盤彙總，避免單筆警告洗版的測試。

use super::*;

/// 成功率、慢回應、p99、矛盾列與代號固定，跨日不混資料。
#[test]
fn summary_and_day_rollover() {
    let mut stats = DailyStats::default();
    let date = NaiveDate::from_ymd_opt(2026, 10, 27).unwrap();
    stats.on_day(date);
    stats.cycle(true, 500);
    stats.cycle(false, 15_000);
    stats.inconsistent_row("2330", Inconsistency::OutOfRange);
    stats.inconsistent_row("0050", Inconsistency::ChangeMismatch);
    stats.inconsistent_row("2330", Inconsistency::ChangeMismatch);
    assert_eq!(
        stats.line(),
        "HiStock 收盤統計：rounds=2, succeeded=1, success=50.0%, slow=1, p99=15000ms, inconsistent=3(高低 1、漲跌 2、別檔 0), dirty_batches=0, symbols=0050,2330"
    );
    let at = |h, m, s| NaiveTime::from_hms_opt(h, m, s).unwrap();
    stats.dirty_batch(at(9, 2, 3));
    stats.dirty_batch(at(9, 11, 20));
    stats.dirty_batch(at(9, 5, 0));
    stats.inconsistent_row("6488", Inconsistency::Misaligned);
    assert!(
        stats.line().contains(
            "inconsistent=4(高低 1、漲跌 2、別檔 1), dirty_batches=3(09:02:03～09:11:20)"
        )
    );
    stats.on_day(date.succ_opt().unwrap());
    assert_eq!(stats.rounds, 0);
    assert!(stats.symbols.is_empty());
}

/// 真正的統計入口會累計資料，收盤只輸出及清空一次。
#[test]
fn shared_stats_are_flushed_once() {
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::fmt().with_test_writer().finish());
    record_cycle(true, 500);
    record_inconsistent([("79979O2", Inconsistency::OutOfRange)].into_iter());
    record_dirty_batch();
    assert!(DAILY.lock().unwrap().dirty_batches > 0);
    assert!(DAILY.lock().unwrap().rounds > 0);
    summarize();
    assert_eq!(DAILY.lock().unwrap().rounds, 0);
    summarize();
}
