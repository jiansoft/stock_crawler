use super::*;

/// 測試用的時間點。
fn at(day: u32, hour: u32, minute: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 10, day)
        .unwrap()
        .and_hms_opt(hour, minute, 0)
        .unwrap()
}

/// 一切正常的樣本：可用 700 MB、swap 用 50%、fsync 12 ms。
fn healthy() -> Sample {
    Sample {
        throttled: Some(0),
        memory: Some(MemInfo {
            available_kib: 700 * 1024,
            swap_total_kib: 200 * 1024,
            swap_free_kib: 100 * 1024,
        }),
        probe: Probe::Done(Duration::from_millis(12)),
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

#[test]
fn daily_stats_summarize_extremes_and_reset_on_new_day() {
    let mut stats = DailyStats::default();
    assert_eq!(stats.summary(at(27, 23, 30).date()), None);

    stats.record(at(27, 0, 0).date(), &healthy());
    stats.record(
        at(27, 0, 10).date(),
        &Sample {
            throttled: Some(0x50004),
            memory: Some(MemInfo {
                available_kib: 650 * 1024,
                swap_total_kib: 200 * 1024,
                swap_free_kib: 20 * 1024,
            }),
            probe: Probe::Done(Duration::from_millis(340)),
        },
    );
    stats.record(
        at(27, 0, 20).date(),
        &Sample {
            throttled: None,
            memory: None,
            probe: Probe::TimedOut,
        },
    );
    assert_eq!(
        stats.summary(at(27, 23, 30).date()).unwrap(),
        "主機健康彙總 2026-10-27：檢查 3 次｜fsync 最大 340 ms、逾時或失敗 1 次｜MemAvailable 最低 650 MB｜swap 最高 90.0%｜供電／降頻 降頻"
    );
    // 別天的彙總不輸出。
    assert_eq!(stats.summary(at(28, 23, 30).date()), None);

    // 換日後重新累計；探針略過、沒有記憶體資料時顯示「略過」與「—」。
    stats.record(
        at(28, 0, 0).date(),
        &Sample {
            throttled: None,
            memory: None,
            probe: Probe::Skipped,
        },
    );
    assert_eq!(
        stats.summary(at(28, 23, 30).date()).unwrap(),
        "主機健康彙總 2026-10-28：檢查 1 次｜fsync 略過｜MemAvailable 最低 —｜swap 最高 —｜供電／降頻 無"
    );

    stats.record(at(29, 0, 0).date(), &healthy());
    assert_eq!(
        stats.summary(at(29, 23, 30).date()).unwrap(),
        "主機健康彙總 2026-10-29：檢查 1 次｜fsync 最大 12 ms｜MemAvailable 最低 700 MB｜swap 最高 50.0%｜供電／降頻 無"
    );
}

/// 告警依冷卻時間去重，彙總照常累計每一次檢查。
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
    assert_eq!(stats.checks, 3);
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
    let probing = Arc::new(AtomicBool::new(false));
    let paths = Paths {
        throttled: dir.0.join("get_throttled"),
        meminfo: dir.0.join("meminfo"),
        probe: dir.0.join(".probe"),
    };

    let sample = collect(&paths, &probing, Duration::from_secs(30)).await;
    assert_eq!(sample.throttled, Some(0x50000));
    assert_eq!(sample.memory.map(|m| m.available_kib), Some(512_000));
    assert!(matches!(sample.probe, Probe::Done(_)), "{:?}", sample.probe);
    assert_eq!(std::fs::metadata(&paths.probe).unwrap().len(), 4096);
    assert!(!probing.load(Ordering::SeqCst), "探針結束後要清除旗標");

    let missing = Paths {
        throttled: dir.0.join("none"),
        meminfo: dir.0.join("none"),
        probe: dir.0.join("no_such_dir").join(".probe"),
    };
    let sample = collect(&missing, &probing, Duration::from_secs(30)).await;
    assert_eq!(
        sample,
        Sample {
            throttled: None,
            memory: None,
            probe: Probe::Skipped,
        }
    );
    assert!(Paths::default().throttled.ends_with("get_throttled"));
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
