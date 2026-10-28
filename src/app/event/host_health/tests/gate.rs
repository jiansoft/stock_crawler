//! 告警冷卻、開機以來位元與每日彙總的測試。

use super::*;

/// 同一類異常 6 小時內只告警一次，不同類別互不影響。
#[test]
fn alert_gate_suppresses_same_kind_for_six_hours() {
    let mut gate = AlertGate::default();
    assert!(gate.should_alert(Kind::Storage, at(27, 2, 0)));
    assert!(!gate.should_alert(Kind::Storage, at(27, 2, 10)));
    assert!(gate.should_alert(Kind::Power, at(27, 2, 10)));
    assert!(!gate.should_alert(Kind::Storage, at(27, 7, 59)));
    assert!(gate.should_alert(Kind::Storage, at(27, 8, 0)));
    assert!(!gate.should_alert(Kind::Storage, at(27, 13, 50)));
}

/// 開機以來位元：第一次讀到只記 info，之後由 0 變 1 的位元只記一次 warn。
#[test]
fn since_boot_flags_warn_once_per_new_bit() {
    let mut gate = AlertGate::default();
    let with = |value| Sample {
        since_boot: Some(value),
        ..healthy()
    };
    assert_eq!(note_since_boot(&empty(Probe::Skipped), &mut gate), None);
    assert_eq!(
        note_since_boot(&with(0x50000), &mut gate).unwrap(),
        "開機以來曾供電不足、曾降頻（vcgencmd get_throttled=0x50000）"
    );
    assert_eq!(note_since_boot(&with(0x50000), &mut gate), None);
    assert_eq!(
        note_since_boot(&with(0x70000), &mut gate).unwrap(),
        "開機以來新出現曾頻率受限（vcgencmd get_throttled=0x70000）；發生在上次檢查之後，確切時間請查 `dmesg`"
    );
    assert_eq!(note_since_boot(&with(0x70000), &mut gate), None);

    // 行程啟動時開機以來沒有任何異常：不記；之後出現才 warn。
    let mut fresh = AlertGate::default();
    assert_eq!(note_since_boot(&with(0x0), &mut fresh), None);
    assert!(
        note_since_boot(&with(0x10001), &mut fresh)
            .unwrap()
            .starts_with("開機以來新出現曾供電不足（vcgencmd get_throttled=0x10001）")
    );
}

#[test]
fn daily_stats_summarize_extremes_and_reset_on_new_day() {
    let mut stats = DailyStats::default();
    assert_eq!(stats.summary(at(27, 23, 30).date()), None);

    stats.record(at(27, 0, 0).date(), &healthy(), 0);
    stats.record(
        at(27, 0, 10).date(),
        &Sample {
            throttled: Some(0x50004),
            since_boot: Some(0x70000),
            memory: Some(MemInfo {
                available_kib: 650 * 1024,
                swap_total_kib: 200 * 1024,
                swap_free_kib: 20 * 1024,
            }),
            probe: Probe::Done(Duration::from_millis(340)),
            deploy: None,
        },
        0,
    );
    stats.record(at(27, 0, 20).date(), &empty(Probe::TimedOut), 0);
    assert_eq!(
        stats.summary(at(27, 23, 30).date()).unwrap(),
        "主機健康彙總 2026-10-27：檢查 3 次｜fsync 最大 340 ms、逾時或失敗 1 次｜MemAvailable 最低 650 MB｜swap 最高 90.0%｜供電／降頻 目前 降頻；開機以來 曾供電不足、曾頻率受限、曾降頻"
    );
    // 別天的彙總不輸出。
    assert_eq!(stats.summary(at(28, 23, 30).date()), None);

    // 換日後重新累計；探針略過、沒有記憶體與 vcgencmd 資料時顯示「略過」與「—」。
    stats.record(at(28, 0, 0).date(), &empty(Probe::Skipped), 0);
    assert_eq!(
        stats.summary(at(28, 23, 30).date()).unwrap(),
        "主機健康彙總 2026-10-28：檢查 1 次｜fsync 略過｜MemAvailable 最低 —｜swap 最高 —｜供電／降頻 目前 無；開機以來 —"
    );

    // 部署期間的檢查次數與略過的告警項數另列；過期標記不算部署期間。
    stats.record(at(29, 0, 0).date(), &healthy(), 0);
    stats.record(at(29, 8, 10).date(), &deploying(marker(3, false)), 2);
    stats.record(at(29, 8, 20).date(), &deploying(marker(12, true)), 1);
    stats.record(at(29, 9, 0).date(), &deploying(marker(52, true)), 0);
    assert_eq!(
        stats.summary(at(29, 23, 30).date()).unwrap(),
        "主機健康彙總 2026-10-29：檢查 4 次｜fsync 最大 22600 ms｜MemAvailable 最低 300 MB｜swap 最高 100.0%｜供電／降頻 目前 無；開機以來 無｜部署期間 2 次（略過告警 3 項）"
    );
}

/// 告警依冷卻時間去重，部署期間不告警，彙總照常累計每一次檢查。
#[test]
fn plan_alerts_records_stats_and_deduplicates() {
    let mut gate = AlertGate::default();
    let mut stats = DailyStats::default();
    let bad = Sample {
        throttled: Some(0x1),
        probe: Probe::Done(Duration::from_secs(6)),
        ..healthy()
    };

    let first = plan_alerts(&bad, at(27, 2, 0), &mut gate, &mut stats);
    assert_eq!(first.len(), 1);
    assert!(first[0].starts_with("目前供電不足"), "{first:?}");
    assert!(plan_alerts(&bad, at(27, 2, 10), &mut gate, &mut stats).is_empty());
    assert!(plan_alerts(&healthy(), at(27, 2, 20), &mut gate, &mut stats).is_empty());
    assert!(
        plan_alerts(
            &deploying(marker(1, false)),
            at(27, 2, 30),
            &mut gate,
            &mut stats
        )
        .is_empty()
    );
    assert_eq!(stats.checks, 4);
    assert_eq!(stats.deploy_checks, 1);
    assert_eq!(stats.suppressed, 2);
    assert_eq!(gate.since_boot, Some(0));
}
