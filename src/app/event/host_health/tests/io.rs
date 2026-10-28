//! 實際讀檔、執行 `vcgencmd`、寫入探針與排程入口的測試。

use super::*;

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
