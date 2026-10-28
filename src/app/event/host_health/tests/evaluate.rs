//! 單次樣本的判斷規則：供電、記憶體、SD 卡與部署期間的告警與 warn。

use super::*;

#[test]
fn healthy_sample_has_no_findings() {
    assert_eq!(evaluate(&healthy()), Findings::default());
}

/// 只有「目前」的供電不足（bit 0）或降頻（bit 2）告警；開機以來曾經發生（bit 16 起）不告警。
#[test]
fn power_alerts_only_on_current_under_voltage_or_throttling() {
    let with = |flags| Sample {
        throttled: Some(flags),
        ..healthy()
    };
    assert!(evaluate(&with(0x0)).alerts.is_empty());
    assert!(evaluate(&with(0x50000)).alerts.is_empty());
    assert!(evaluate(&with(0x70000)).alerts.is_empty());
    // 只有頻率受限（bit 1）不告警。
    assert!(evaluate(&with(0x2)).alerts.is_empty());
    // vcgencmd 的開機以來位元不告警。
    assert!(
        evaluate(&Sample {
            since_boot: Some(0x70000),
            ..healthy()
        })
        .alerts
        .is_empty()
    );

    let under_voltage = evaluate(&with(0x1));
    assert_eq!(under_voltage.alerts.len(), 1);
    assert_eq!(under_voltage.alerts[0].0, Kind::Power);
    assert!(
        under_voltage.alerts[0]
            .1
            .starts_with("目前供電不足（get_throttled=0x1）"),
        "{:?}",
        under_voltage.alerts
    );

    let throttled = evaluate(&with(0x4));
    assert!(
        throttled.alerts[0]
            .1
            .starts_with("目前降頻（get_throttled=0x4）"),
        "{:?}",
        throttled.alerts
    );

    let all = evaluate(&with(0x5000f));
    assert!(
        all.alerts[0]
            .1
            .starts_with("目前供電不足、頻率受限、降頻、溫度達軟上限（get_throttled=0x5000f）"),
        "{:?}",
        all.alerts
    );
}

#[test]
fn memory_alerts_on_low_available_or_full_swap() {
    let with = |available_mb: u64, swap_free_mb: u64| Sample {
        memory: Some(MemInfo {
            available_kib: available_mb * 1024,
            swap_total_kib: 200 * 1024,
            swap_free_kib: swap_free_mb * 1024,
        }),
        ..healthy()
    };
    // 門檻本身不告警：剛好 100 MB、swap 剛好 95%。
    assert!(evaluate(&with(100, 10)).alerts.is_empty());

    let low = evaluate(&with(99, 100));
    assert_eq!(
        low.alerts,
        vec![(Kind::Memory, "可用記憶體 99 MB（門檻 100 MB）".to_owned())]
    );

    let swap = evaluate(&with(500, 2));
    assert_eq!(
        swap.alerts,
        vec![(
            Kind::Memory,
            "swap 已用 99.0%（198／200 MB，門檻 95%）".to_owned()
        )]
    );

    // 兩者同時發生合成一則。
    let both = evaluate(&with(50, 2));
    assert_eq!(both.alerts.len(), 1);
    assert_eq!(
        both.alerts[0].1,
        "可用記憶體 50 MB（門檻 100 MB）；swap 已用 99.0%（198／200 MB，門檻 95%）"
    );
}

#[test]
fn storage_thresholds_split_warn_and_alert() {
    let with = |probe| Sample { probe, ..healthy() };
    assert_eq!(evaluate(&with(Probe::Skipped)), Findings::default());
    assert_eq!(
        evaluate(&with(Probe::Done(Duration::from_millis(4_999)))),
        Findings::default()
    );

    let slow = evaluate(&with(Probe::Done(Duration::from_secs(5))));
    assert!(slow.alerts.is_empty());
    assert_eq!(
        slow.warnings,
        vec!["SD 卡寫入 4 KB 並 fsync 耗時 5.0 秒（warn 門檻 5 秒）".to_owned()]
    );

    let stuck = evaluate(&with(Probe::Done(Duration::from_millis(25_300))));
    assert!(stuck.warnings.is_empty());
    assert_eq!(stuck.alerts.len(), 1);
    assert_eq!(stuck.alerts[0].0, Kind::Storage);
    assert!(
        stuck.alerts[0]
            .1
            .starts_with("SD 卡寫入 4 KB 並 fsync 耗時 25.3 秒（門檻 20 秒）"),
        "{:?}",
        stuck.alerts
    );

    for (probe, prefix) in [
        (
            Probe::TimedOut,
            "SD 卡寫入 4 KB 並 fsync 超過 60 秒仍未完成",
        ),
        (Probe::StillRunning, "上一輪 SD 卡寫入探針仍未完成"),
        (
            Probe::Failed("Read-only file system".to_owned()),
            "SD 卡寫入探針失敗：Read-only file system",
        ),
    ] {
        let findings = evaluate(&with(probe));
        assert_eq!(findings.alerts.len(), 1);
        assert_eq!(findings.alerts[0].0, Kind::Storage);
        assert!(
            findings.alerts[0].1.starts_with(prefix),
            "{:?}",
            findings.alerts
        );
    }
}

/// 部署期間（含結束後 30 分鐘）記憶體與 SD 卡超標不告警、warn 也改 info；供電照常告警。
#[test]
fn deploy_marker_suppresses_memory_and_storage_alerts() {
    for deploy in [marker(0, false), marker(29, true), marker(30, false)] {
        let findings = evaluate(&deploying(deploy.clone()));
        assert!(findings.alerts.is_empty(), "{deploy:?}: {findings:?}");
        assert!(findings.warnings.is_empty(), "{deploy:?}: {findings:?}");
        assert_eq!(findings.suppressed.len(), 2, "{deploy:?}: {findings:?}");
        assert!(findings.suppressed[0].starts_with("swap 已用 100.0%"));
        assert!(findings.suppressed[1].starts_with("SD 卡寫入 4 KB 並 fsync 耗時 22.6 秒"));
    }

    // fsync warn 也一併略過。
    let slow = evaluate(&Sample {
        probe: Probe::Done(Duration::from_secs(6)),
        deploy: marker(5, false),
        ..healthy()
    });
    assert_eq!(slow.warnings, Vec::<String>::new());
    assert_eq!(slow.suppressed.len(), 1);

    let power = evaluate(&Sample {
        throttled: Some(0x1),
        ..deploying(marker(5, false))
    });
    assert_eq!(power.alerts.len(), 1);
    assert_eq!(power.alerts[0].0, Kind::Power);
    assert_eq!(power.suppressed.len(), 2);
}

/// 已結束又超過 30 分鐘的標記不影響；沒寫入結束又超過 30 分鐘是殘留，照常告警並在 warn 註明。
#[test]
fn expired_or_stale_marker_alerts_normally() {
    let finished = evaluate(&deploying(marker(31, true)));
    assert_eq!(finished.alerts.len(), 2);
    assert!(finished.warnings.is_empty());
    assert!(finished.suppressed.is_empty());

    let stale = evaluate(&deploying(marker(45, false)));
    assert_eq!(stale.alerts.len(), 2);
    assert!(stale.suppressed.is_empty());
    assert_eq!(
        stale.warnings,
        vec!["部署標記 stock_go-web 已 45 分鐘沒有寫入結束，視為殘留，照常告警".to_owned()]
    );

    // 殘留標記但一切正常時不多記。
    assert_eq!(
        evaluate(&Sample {
            deploy: marker(45, false),
            ..healthy()
        }),
        Findings::default()
    );
}
