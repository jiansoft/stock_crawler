use super::*;

/// 測試用的時間點。
fn at(day: u32, hour: u32, minute: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 10, day)
        .unwrap()
        .and_hms_opt(hour, minute, 0)
        .unwrap()
}

/// 一切正常的樣本：可用 700 MB、swap 用 50%、fsync 12 ms、開機以來沒有供電異常、不在部署期間。
fn healthy() -> Sample {
    Sample {
        throttled: Some(0),
        since_boot: Some(0),
        memory: Some(MemInfo {
            available_kib: 700 * 1024,
            swap_total_kib: 200 * 1024,
            swap_free_kib: 100 * 1024,
        }),
        probe: Probe::Done(Duration::from_millis(12)),
        deploy: None,
    }
}

/// 讀不到任何資料的樣本。
fn empty(probe: Probe) -> Sample {
    Sample {
        throttled: None,
        since_boot: None,
        memory: None,
        probe,
        deploy: None,
    }
}

/// 部署標記：`minutes` 分鐘前最後異動，`finished` 表示腳本已寫入結束。
fn marker(minutes: u64, finished: bool) -> Option<DeployMarker> {
    Some(DeployMarker {
        unit: "stock_go-web".to_owned(),
        age: Duration::from_secs(minutes * 60),
        finished,
    })
}

/// 部署時常見的樣本：swap 用滿、fsync 22.6 秒。
fn deploying(deploy: Option<DeployMarker>) -> Sample {
    Sample {
        memory: Some(MemInfo {
            available_kib: 300 * 1024,
            swap_total_kib: 200 * 1024,
            swap_free_kib: 0,
        }),
        probe: Probe::Done(Duration::from_millis(22_600)),
        deploy,
        ..healthy()
    }
}

/// 測試專用的暫存目錄（每個測試不同名稱，結束時刪除）。
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "stock_crawler_host_health_{name}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn parse_throttled_accepts_bare_and_prefixed_hex() {
    assert_eq!(parse_throttled("0\n"), Some(0));
    assert_eq!(parse_throttled("50005\n"), Some(0x50005));
    assert_eq!(parse_throttled("0x70000"), Some(0x70000));
    assert_eq!(parse_throttled("0X4"), Some(4));
    assert_eq!(parse_throttled("throttled=0x0"), None);
    assert_eq!(parse_throttled(""), None);
}

#[test]
fn parse_vcgencmd_reads_throttled_line() {
    assert_eq!(
        parse_vcgencmd_throttled("throttled=0x70000\n"),
        Some(0x70000)
    );
    assert_eq!(parse_vcgencmd_throttled("throttled=0x0"), Some(0));
    assert_eq!(parse_vcgencmd_throttled("0x70000"), None);
    assert_eq!(parse_vcgencmd_throttled("throttled=zz"), None);
}

#[test]
fn parse_meminfo_reads_available_and_swap() {
    let text = "MemTotal:         945364 kB\nMemFree:           48000 kB\nMemAvailable:     705164 kB\nSwapTotal:        204796 kB\nSwapFree:          42060 kB\n";
    let info = parse_meminfo(text).unwrap();
    assert_eq!(
        info,
        MemInfo {
            available_kib: 705_164,
            swap_total_kib: 204_796,
            swap_free_kib: 42_060,
        }
    );
    let percent = info.swap_used_percent().unwrap();
    assert!((percent - 79.46).abs() < 0.01, "{percent}");

    // 沒有 swap 欄位視為沒有 swap；缺 MemAvailable 無法判斷。
    let no_swap = parse_meminfo("MemAvailable: 1024 kB\n").unwrap();
    assert_eq!(no_swap.swap_used_percent(), None);
    assert_eq!(parse_meminfo("MemTotal: 1024 kB\n"), None);
}

/// 標記內容的 `unit=` 優先，沒有就用檔名；有 `finished=` 才算已結束。
#[test]
fn parse_deploy_marker_reads_unit_and_finished() {
    let age = Duration::from_secs(90);
    assert_eq!(
        parse_deploy_marker(
            "unit=stock_go\nstarted=2026-10-28T08:05:00+08:00\n",
            "file",
            age
        ),
        DeployMarker {
            unit: "stock_go".to_owned(),
            age,
            finished: false,
        }
    );
    let done = parse_deploy_marker(
        "unit=stock-mcp\r\nstarted=x\r\nfinished=2026-10-28T08:10:00+08:00\r\n",
        "file",
        age,
    );
    assert_eq!(done.unit, "stock-mcp");
    assert!(done.finished);
    assert_eq!(
        parse_deploy_marker("unit=\n", "stock_crawler", age).unit,
        "stock_crawler"
    );
    assert_eq!(
        parse_deploy_marker("", "stock_crawler", age).unit,
        "stock_crawler"
    );
}

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

/// 讀取暫存檔與實際寫入探針：檔案不存在略過，目錄不存在不寫入。
#[tokio::test]
async fn collect_reads_files_and_probes_storage() {
    let dir = TempDir::new("collect");
    std::fs::write(dir.0.join("get_throttled"), "50000\n").unwrap();
    std::fs::write(
        dir.0.join("meminfo"),
        "MemAvailable: 512000 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.0.join("deploying")).unwrap();
    std::fs::write(
        dir.0.join("deploying").join("stock_crawler"),
        "unit=stock_crawler\nstarted=2026-10-28T08:00:00+08:00\n",
    )
    .unwrap();
    let probing = Arc::new(AtomicBool::new(false));
    let paths = Paths {
        throttled: dir.0.join("get_throttled"),
        vcgencmd: dir.0.join("none"),
        meminfo: dir.0.join("meminfo"),
        probe: dir.0.join(".probe"),
        deploy_markers: dir.0.join("deploying"),
    };

    let sample = collect(&paths, &probing, Duration::from_secs(30)).await;
    assert_eq!(sample.throttled, Some(0x50000));
    assert_eq!(sample.since_boot, None);
    assert_eq!(sample.memory.map(|m| m.available_kib), Some(512_000));
    assert!(matches!(sample.probe, Probe::Done(_)), "{:?}", sample.probe);
    assert_eq!(std::fs::metadata(&paths.probe).unwrap().len(), 4096);
    assert!(!probing.load(Ordering::SeqCst), "探針結束後要清除旗標");
    let deploy = sample.deploy.expect("讀到部署標記");
    assert_eq!(deploy.unit, "stock_crawler");
    assert!(deploy.active() && !deploy.finished, "{deploy:?}");

    let missing = Paths {
        throttled: dir.0.join("none"),
        vcgencmd: dir.0.join("none"),
        meminfo: dir.0.join("none"),
        probe: dir.0.join("no_such_dir").join(".probe"),
        deploy_markers: dir.0.join("none"),
    };
    let sample = collect(&missing, &probing, Duration::from_secs(30)).await;
    assert_eq!(sample, empty(Probe::Skipped));
    let defaults = Paths::default();
    assert!(defaults.throttled.ends_with("get_throttled"));
    assert!(defaults.vcgencmd.ends_with("vcgencmd"));
    assert!(defaults.deploy_markers.starts_with("/dev/shm"));
}

/// 有多個標記時取最近異動的一個；子目錄忽略，空目錄回 `None`。
#[test]
fn read_deploy_marker_picks_the_most_recent() {
    let dir = TempDir::new("markers");
    assert_eq!(read_deploy_marker(&dir.0, SystemTime::now()), None);

    std::fs::create_dir_all(dir.0.join("nested")).unwrap();
    let old = dir.0.join("stock_go");
    std::fs::write(&old, "unit=stock_go\nstarted=x\nfinished=y\n").unwrap();
    let now = SystemTime::now();
    std::fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(now - Duration::from_secs(40 * 60))
        .unwrap();
    let only_old = read_deploy_marker(&dir.0, now).unwrap();
    assert_eq!(only_old.unit, "stock_go");
    assert!(only_old.finished && !only_old.active());

    std::fs::write(dir.0.join("stock_go-web"), "started=x\n").unwrap();
    let newest = read_deploy_marker(&dir.0, SystemTime::now()).unwrap();
    assert_eq!(newest.unit, "stock_go-web");
    assert!(!newest.finished && newest.active());
}

/// `vcgencmd` 不存在時不執行；指令輸出成功、失敗與格式不符分別處理。
#[cfg(unix)]
#[tokio::test]
async fn read_since_boot_runs_vcgencmd() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new("vcgencmd");
    assert_eq!(
        read_since_boot(&dir.0.join("none"), Duration::from_secs(5)).await,
        None
    );
    let script = |name: &str, body: &str| {
        let path = dir.0.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    };
    let ok = script(
        "ok",
        r#"[ "$1" = get_throttled ] && echo throttled=0x70000"#,
    );
    assert_eq!(
        read_since_boot(&ok, Duration::from_secs(5)).await,
        Some(0x70000)
    );
    let failed = script("failed", "exit 3");
    assert_eq!(read_since_boot(&failed, Duration::from_secs(5)).await, None);
    let garbage = script("garbage", "echo nope");
    assert_eq!(
        read_since_boot(&garbage, Duration::from_secs(5)).await,
        None
    );
    let slow = script("slow", "sleep 5");
    assert_eq!(
        read_since_boot(&slow, Duration::from_millis(200)).await,
        None
    );
    // 不是可執行檔：執行失敗。
    let not_executable = dir.0.join("plain");
    std::fs::write(&not_executable, "throttled=0x0").unwrap();
    assert_eq!(
        read_since_boot(&not_executable, Duration::from_secs(5)).await,
        None
    );
}

/// 上一輪仍在寫入時不再送出新的探針；寫入失敗回報錯誤。
#[tokio::test]
async fn probe_storage_reports_still_running_and_failures() {
    let dir = TempDir::new("probe");
    let busy = Arc::new(AtomicBool::new(true));
    assert_eq!(
        probe_storage(&dir.0.join(".probe"), &busy, Duration::from_secs(30)).await,
        Probe::StillRunning
    );

    // 探針路徑是既有目錄時建立檔案會失敗。
    std::fs::create_dir_all(dir.0.join("occupied")).unwrap();
    let idle = Arc::new(AtomicBool::new(false));
    let result = probe_storage(&dir.0.join("occupied"), &idle, Duration::from_secs(30)).await;
    assert!(matches!(result, Probe::Failed(_)), "{result:?}");
    assert!(!idle.load(Ordering::SeqCst));
}

/// 排程入口不會失敗；彙總在沒有當日資料時不輸出也不報錯。
#[tokio::test]
async fn execute_and_summarize_never_fail() {
    execute().await.unwrap();
    summarize().await.unwrap();
}
