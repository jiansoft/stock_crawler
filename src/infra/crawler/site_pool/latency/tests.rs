//! 站點延遲統計的測試。

use super::*;

/// 驗證站點延遲統計可以正確計算平均值與各主要百分位。
#[test]
fn test_site_latency_stats_average_and_percentiles() {
    let mut stats = SiteLatencyStats::default();

    for elapsed_ms in [10, 20, 30, 40, 50] {
        stats.record(elapsed_ms);
    }

    assert_eq!(stats.sample_count(), 5);
    assert_eq!(stats.average_ms(), 30);
    assert_eq!(stats.p50_ms(), 30);
    assert_eq!(stats.p70_ms(), 40);
    assert_eq!(stats.p99_ms(), 50);
}

/// 成敗與熔斷次數各自累計；失敗率超過 10% 且樣本至少 20 筆才記 WARN。
#[test]
fn site_results_count_failures_and_trips() {
    let mut stats = SiteLatencyStats::default();
    stats.record_result(10, true, false);
    stats.record_result(30, false, false);
    stats.record_result(20, false, true);
    assert_eq!(
        (stats.sample_count(), stats.failures, stats.trips),
        (3, 2, 1)
    );
    assert_eq!(stats.average_ms(), 20);

    assert!(!failure_needs_attention(19, 19));
    assert!(!failure_needs_attention(20, 2));
    assert!(failure_needs_attention(20, 3));
    assert!(failure_needs_attention(1_000, 101));
    assert!(!failure_needs_attention(1_000, 100));

    let line = summary_line(&SiteLatencySnapshot {
        site_name: "PcHome",
        count: 800,
        failures: 70,
        trips: 3,
        skipped: 0,
        avg_ms: 1_200,
        p50_ms: 900,
        p70_ms: 1_100,
        p99_ms: 6_000,
    });
    assert_eq!(
        line,
        "站點整體耗時統計 PcHome: count=800, failed=70(8.8%), trips=3, avg=1200ms, p50=900ms, p70=1100ms, p99=6000ms"
    );
    let empty = summary_line(&SiteLatencySnapshot {
        site_name: "Empty",
        count: 0,
        failures: 0,
        trips: 0,
        skipped: 0,
        avg_ms: 0,
        p50_ms: 0,
        p70_ms: 0,
        p99_ms: 0,
    });
    assert!(empty.contains("failed=0(0.0%)"), "{empty}");
}

/// 略過次數另計：不算取樣、不算失敗；統計行只在有略過時多印 skipped。
#[test]
fn skipped_requests_are_counted_separately() {
    record_site_skipped("TestSkippedOnly");
    record_site_skipped("TestSkippedOnly");
    {
        let all_stats = SITE_LATENCY_STATS.lock().expect("lock site latency stats");
        let stats = all_stats.get("TestSkippedOnly").expect("略過要建立統計");
        assert_eq!(
            (stats.sample_count(), stats.failures, stats.skipped),
            (0, 0, 2)
        );
    }
    SITE_LATENCY_STATS
        .lock()
        .expect("lock site latency stats")
        .remove("TestSkippedOnly");

    let line = summary_line(&SiteLatencySnapshot {
        site_name: "Fugle",
        count: 15_215,
        failures: 20,
        trips: 0,
        skipped: 1_033,
        avg_ms: 270,
        p50_ms: 234,
        p70_ms: 343,
        p99_ms: 1_096,
    });
    assert_eq!(
        line,
        "站點整體耗時統計 Fugle: count=15215, failed=20(0.1%), trips=0, skipped=1033, avg=270ms, p50=234ms, p70=343ms, p99=1096ms"
    );
}

/// 失敗率過半時註明建議移出站點池（10-28 PcHome 275 次失敗 215 次），10%～50% 只標示超過 10%。
#[test]
fn failure_note_suggests_removal_above_half() {
    assert_eq!(failure_note(275, 215), "（失敗率超過 50%，建議移出站點池）");
    assert_eq!(failure_note(100, 50), "（失敗率超過 10%）");
    assert_eq!(failure_note(100, 11), "（失敗率超過 10%）");
}

/// 驗證站點延遲統計在單一樣本時仍能正確回傳平均值與各主要百分位。
#[test]
fn test_site_latency_stats_percentiles_with_single_sample() {
    let mut stats = SiteLatencyStats::default();
    stats.record(88);

    assert_eq!(stats.sample_count(), 1);
    assert_eq!(stats.average_ms(), 88);
    assert_eq!(stats.p50_ms(), 88);
    assert_eq!(stats.p70_ms(), 88);
    assert_eq!(stats.p99_ms(), 88);
}

/// 驗證輸出延遲統計後，累積中的站點資料會被清空。
#[test]
fn test_flush_site_latency_stats_clears_data() {
    // 開訂閱者讓統計行的格式化實際被求值。
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    {
        let mut all_stats = SITE_LATENCY_STATS.lock().expect("lock site latency stats");
        all_stats.clear();
        all_stats.entry("Yahoo").or_default().record(12);
        all_stats.entry("Fugle").or_default().record(34);
        // 失敗率 20%：收盤那一行走 WARN 分支。
        let flaky = all_stats.entry("TestFlaky").or_default();
        for idx in 0..25 {
            flaky.record_result(10, idx % 5 != 0, idx == 0);
        }
    }

    flush_site_latency_stats();

    // 只檢查本測試寫入的站點：其他測試的假站點可能在 flush 之後並行寫入。
    let all_stats = SITE_LATENCY_STATS.lock().expect("lock site latency stats");
    assert!(!all_stats.contains_key("Yahoo"));
    assert!(!all_stats.contains_key("Fugle"));
    assert!(!all_stats.contains_key("TestFlaky"));
}
