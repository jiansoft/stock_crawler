//! 取樣：讀取各檔案、執行 `vcgencmd`、寫入 SD 卡探針，組成一次檢查的 [`super::Sample`]。

use super::*;

/// 檢查用到的檔案路徑；測試可換成暫存檔。
#[derive(Debug, Clone)]
pub(crate) struct Paths {
    pub(crate) throttled: PathBuf,
    pub(crate) vcgencmd: PathBuf,
    pub(crate) meminfo: PathBuf,
    pub(crate) probe: PathBuf,
    pub(crate) deploy_markers: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            throttled: PathBuf::from(THROTTLED_PATH),
            vcgencmd: PathBuf::from(VCGENCMD_PATH),
            meminfo: PathBuf::from(MEMINFO_PATH),
            probe: PathBuf::from(PROBE_PATH),
            deploy_markers: PathBuf::from(DEPLOY_MARKER_DIR),
        }
    }
}

/// 在 `path` 寫 [`PROBE_BYTES`] 位元組並 fsync，回傳耗時。
pub(super) fn write_probe(path: &Path) -> std::io::Result<Duration> {
    let started = Instant::now();
    let mut file = std::fs::File::create(path)?;
    file.write_all(&[0u8; PROBE_BYTES])?;
    file.sync_all()?;
    Ok(started.elapsed())
}

/// 在背景執行緒寫入探針，最多等 `timeout`；`probing` 為真（上一輪仍卡著）時不再送出。
pub(crate) async fn probe_storage(
    path: &Path,
    probing: &Arc<AtomicBool>,
    timeout: Duration,
) -> Probe {
    if !path.parent().is_some_and(Path::is_dir) {
        return Probe::Skipped;
    }
    if probing.swap(true, Ordering::SeqCst) {
        return Probe::StillRunning;
    }
    let path = path.to_owned();
    let flag = Arc::clone(probing);
    let handle = tokio::task::spawn_blocking(move || {
        let result = write_probe(&path);
        flag.store(false, Ordering::SeqCst);
        result
    });
    match tokio::time::timeout(timeout, handle).await {
        Ok(Ok(Ok(elapsed))) => Probe::Done(elapsed),
        Ok(Ok(Err(error))) => Probe::Failed(error.to_string()),
        Ok(Err(error)) => {
            probing.store(false, Ordering::SeqCst);
            Probe::Failed(error.to_string())
        }
        Err(_) => Probe::TimedOut,
    }
}

/// 讀取一個小檔案（`/proc`、`/sys`、`/dev/shm` 都在記憶體中，不受 SD 卡卡住影響）；不存在或讀不到回 `None`。
pub(super) fn read_small(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// 執行 `vcgencmd get_throttled`，最多等 `timeout`；指令不存在（非 pi）、失敗或逾時回 `None`。
pub(crate) async fn read_since_boot(vcgencmd: &Path, timeout: Duration) -> Option<u32> {
    if !vcgencmd.is_file() {
        return None;
    }
    let output = tokio::process::Command::new(vcgencmd)
        .arg("get_throttled")
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(timeout, output).await {
        Ok(Ok(output)) if output.status.success() => {
            parse_vcgencmd_throttled(&String::from_utf8_lossy(&output.stdout))
        }
        Ok(Ok(output)) => {
            tracing::debug!("vcgencmd get_throttled 結束碼 {:?}", output.status.code());
            None
        }
        Ok(Err(error)) => {
            tracing::debug!("vcgencmd get_throttled 執行失敗: {error}");
            None
        }
        Err(_) => {
            tracing::debug!("vcgencmd get_throttled 超過 {} 秒", timeout.as_secs());
            None
        }
    }
}

/// 讀取一次檢查的資料。
pub(crate) async fn collect(paths: &Paths, probing: &Arc<AtomicBool>, timeout: Duration) -> Sample {
    Sample {
        throttled: read_small(&paths.throttled).and_then(|text| parse_throttled(&text)),
        since_boot: read_since_boot(&paths.vcgencmd, VCGENCMD_TIMEOUT).await,
        memory: read_small(&paths.meminfo).and_then(|text| parse_meminfo(&text)),
        probe: probe_storage(&paths.probe, probing, timeout).await,
        deploy: read_deploy_marker(&paths.deploy_markers, SystemTime::now()),
    }
}
