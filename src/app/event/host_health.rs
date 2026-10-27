//! # 主機健康檢查
//!
//! 正式站跑在 Raspberry Pi 上。2026-10-27 部署時 MCP 的 SIGTERM 晚了 31 秒才被處理，kernel 同時記錄
//! `mmc_rescan blocked for more than 122 seconds`（10-24 也有一次），`get_throttled` 顯示開機以來曾經
//! 供電不足並降頻。這些狀況不會讓任何排程報錯，只會讓部署、日誌寫入與資料庫連線莫名變慢，因此每 10 分鐘
//! 讀一次：
//!
//! - **供電與降頻**：`get_throttled` 的「目前」位元（bit 0 供電不足、bit 2 降頻）。
//! - **記憶體**：`/proc/meminfo` 的 `MemAvailable` 與 swap 使用率。
//! - **SD 卡**：在 `log/` 寫 4 KB 並 `fsync` 的耗時；I/O 卡住時第一個受害的就是日誌與部署。
//!
//! 異常時發 Telegram，同一類異常 [`ALERT_COOLDOWN_HOURS`] 小時內只發一次；每天 23:30 記一行當日彙總
//! （[`summarize`]），供日後比對。只讀取與告警，不調整系統設定。檔案不存在（本機開發、CI）的項目略過。

use std::{
    collections::HashMap,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::Result;
use chrono::{Local, NaiveDate, NaiveDateTime};
use once_cell::sync::Lazy;

use crate::core::alert;

/// 樹莓派韌體回報供電與降頻狀態的 sysfs 檔案（十六進位，不帶 `0x`）。
const THROTTLED_PATH: &str = "/sys/devices/platform/soc/soc:firmware/get_throttled";
/// 系統記憶體資訊。
const MEMINFO_PATH: &str = "/proc/meminfo";
/// SD 卡探針寫入的檔案；與日誌同一個目錄，量到的就是日誌實際寫入的延遲。
const PROBE_PATH: &str = "log/.host_health_probe";
/// 探針每次寫入的位元組數。
const PROBE_BYTES: usize = 4096;

/// `get_throttled` bit 0：目前供電不足。
const UNDER_VOLTAGE_NOW: u32 = 1 << 0;
/// `get_throttled` bit 1：目前 ARM 頻率受限。
const FREQ_CAPPED_NOW: u32 = 1 << 1;
/// `get_throttled` bit 2：目前降頻。
const THROTTLED_NOW: u32 = 1 << 2;
/// `get_throttled` bit 3：目前達溫度軟上限。
const SOFT_TEMP_LIMIT_NOW: u32 = 1 << 3;
/// 會觸發告警的「目前」位元；bit 16 起是「開機以來曾經發生」，重開機前不會清除，不能拿來告警。
const ALERT_FLAGS: u32 = UNDER_VOLTAGE_NOW | THROTTLED_NOW;

/// `MemAvailable` 低於此值（KiB）告警。
///
/// pi 共 923 MB，平常 `MemAvailable` 約 700 MB（10-27 實測 705 MB）；剩 100 MB 時 OOM killer 已不遠。
const MIN_AVAILABLE_KIB: u64 = 100 * 1024;
/// swap 使用率高於此值（%）告警。
///
/// 10-27 實測已用 79%，主要是閒置的 `mysqld` 被換出，本身不代表記憶體不足；用滿才是警訊。
const MAX_SWAP_USED_PERCENT: f64 = 95.0;
/// fsync 耗時達此值記 warn。
const FSYNC_WARN: Duration = Duration::from_secs(5);
/// fsync 耗時達此值告警。
const FSYNC_ALERT: Duration = Duration::from_secs(20);
/// 探針最多等多久；超過就當成卡住（寫入仍在背景進行，下一輪不會再疊一個）。
const PROBE_TIMEOUT: Duration = Duration::from_secs(60);
/// 同一類異常的告警間隔（小時）。
const ALERT_COOLDOWN_HOURS: i64 = 6;

/// 異常類別；告警去重以類別為單位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    /// 供電不足或降頻。
    Power,
    /// 記憶體或 swap 不足。
    Memory,
    /// SD 卡寫入過慢、卡住或失敗。
    Storage,
}

/// `/proc/meminfo` 中用到的欄位（KiB）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct MemInfo {
    pub(crate) available_kib: u64,
    pub(crate) swap_total_kib: u64,
    pub(crate) swap_free_kib: u64,
}

impl MemInfo {
    /// swap 使用率（%）；沒有 swap 時為 `None`。
    pub(crate) fn swap_used_percent(&self) -> Option<f64> {
        (self.swap_total_kib > 0).then(|| {
            let used = self.swap_total_kib.saturating_sub(self.swap_free_kib);
            used as f64 * 100.0 / self.swap_total_kib as f64
        })
    }
}

/// SD 卡探針的結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Probe {
    /// 日誌目錄不存在，略過。
    Skipped,
    /// 寫入並 fsync 完成的耗時。
    Done(Duration),
    /// 超過 [`PROBE_TIMEOUT`] 仍未完成。
    TimedOut,
    /// 上一輪的探針還沒結束（之前逾時的寫入仍卡著）。
    StillRunning,
    /// 寫入失敗（例如檔案系統被改成唯讀）。
    Failed(String),
}

/// 一次檢查讀到的資料；讀不到的項目為 `None`。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Sample {
    pub(crate) throttled: Option<u32>,
    pub(crate) memory: Option<MemInfo>,
    pub(crate) probe: Probe,
}

/// 一次檢查的判斷結果。
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Findings {
    /// 要發告警的異常。
    pub(crate) alerts: Vec<(Kind, String)>,
    /// 只記 warn 的異常。
    pub(crate) warnings: Vec<String>,
}

/// 解析 `get_throttled`（十六進位，可帶 `0x`）；格式不符回 `None`。
pub(crate) fn parse_throttled(text: &str) -> Option<u32> {
    let text = text.trim();
    let digits = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    u32::from_str_radix(digits, 16).ok()
}

/// 解析 `/proc/meminfo`；缺 `MemAvailable` 回 `None`，沒有 swap 欄位視為沒有 swap。
pub(crate) fn parse_meminfo(text: &str) -> Option<MemInfo> {
    let field = |name: &str| {
        text.lines().find_map(|line| {
            let rest = line.strip_prefix(name)?.strip_prefix(':')?;
            rest.split_whitespace().next()?.parse::<u64>().ok()
        })
    };
    Some(MemInfo {
        available_kib: field("MemAvailable")?,
        swap_total_kib: field("SwapTotal").unwrap_or(0),
        swap_free_kib: field("SwapFree").unwrap_or(0),
    })
}

/// `get_throttled` 目前位元的中文說明，依位元順序。
fn throttled_labels(flags: u32) -> Vec<&'static str> {
    [
        (UNDER_VOLTAGE_NOW, "供電不足"),
        (FREQ_CAPPED_NOW, "頻率受限"),
        (THROTTLED_NOW, "降頻"),
        (SOFT_TEMP_LIMIT_NOW, "溫度達軟上限"),
    ]
    .into_iter()
    .filter(|(bit, _)| flags & bit != 0)
    .map(|(_, label)| label)
    .collect()
}

/// KiB 換成 MB（取整）。
fn mb(kib: u64) -> u64 {
    kib / 1024
}

/// 秒數文字（一位小數）。
fn seconds(duration: Duration) -> String {
    format!("{:.1}", duration.as_secs_f64())
}

/// 判斷一次檢查的結果：哪些要告警、哪些只記 warn。
pub(crate) fn evaluate(sample: &Sample) -> Findings {
    let mut findings = Findings::default();

    if let Some(flags) = sample.throttled
        && flags & ALERT_FLAGS != 0
    {
        findings.alerts.push((
            Kind::Power,
            format!(
                "目前{}（get_throttled=0x{flags:x}）；請檢查電源供應器（建議原廠 5.1 V／2.5 A）與線材",
                throttled_labels(flags).join("、")
            ),
        ));
    }

    if let Some(memory) = sample.memory {
        let mut problems = Vec::new();
        if memory.available_kib < MIN_AVAILABLE_KIB {
            problems.push(format!(
                "可用記憶體 {} MB（門檻 {} MB）",
                mb(memory.available_kib),
                mb(MIN_AVAILABLE_KIB)
            ));
        }
        if let Some(percent) = memory.swap_used_percent()
            && percent > MAX_SWAP_USED_PERCENT
        {
            problems.push(format!(
                "swap 已用 {percent:.1}%（{}／{} MB，門檻 {MAX_SWAP_USED_PERCENT:.0}%）",
                mb(memory.swap_total_kib.saturating_sub(memory.swap_free_kib)),
                mb(memory.swap_total_kib)
            ));
        }
        if !problems.is_empty() {
            findings.alerts.push((Kind::Memory, problems.join("；")));
        }
    }

    let storage = |message: String| (Kind::Storage, message);
    match &sample.probe {
        Probe::Skipped => {}
        Probe::Done(elapsed) if *elapsed >= FSYNC_ALERT => {
            findings.alerts.push(storage(format!(
                "SD 卡寫入 4 KB 並 fsync 耗時 {} 秒（門檻 {} 秒）；`dmesg` 若有 mmc 逾時紀錄，建議更換 SD 卡",
                seconds(*elapsed),
                FSYNC_ALERT.as_secs()
            )));
        }
        Probe::Done(elapsed) if *elapsed >= FSYNC_WARN => {
            findings.warnings.push(format!(
                "SD 卡寫入 4 KB 並 fsync 耗時 {} 秒（warn 門檻 {} 秒）",
                seconds(*elapsed),
                FSYNC_WARN.as_secs()
            ));
        }
        Probe::Done(_) => {}
        Probe::TimedOut => findings.alerts.push(storage(format!(
            "SD 卡寫入 4 KB 並 fsync 超過 {} 秒仍未完成；I/O 可能卡住，部署與日誌寫入會受影響",
            PROBE_TIMEOUT.as_secs()
        ))),
        Probe::StillRunning => findings.alerts.push(storage(
            "上一輪 SD 卡寫入探針仍未完成；I/O 可能卡住，部署與日誌寫入會受影響".to_owned(),
        )),
        Probe::Failed(error) => findings.alerts.push(storage(format!(
            "SD 卡寫入探針失敗：{error}；檔案系統可能已被改成唯讀"
        ))),
    }

    findings
}

/// 各類異常最近一次發告警的時間。
#[derive(Debug, Default)]
pub(crate) struct AlertGate {
    last_sent: HashMap<Kind, NaiveDateTime>,
}

impl AlertGate {
    /// `kind` 這次是否該發告警（距上次超過冷卻時間）；該發時記下時間。
    pub(crate) fn should_alert(&mut self, kind: Kind, now: NaiveDateTime) -> bool {
        if let Some(last) = self.last_sent.get(&kind)
            && now.signed_duration_since(*last) < chrono::Duration::hours(ALERT_COOLDOWN_HOURS)
        {
            return false;
        }
        self.last_sent.insert(kind, now);
        true
    }
}

/// 當日的檢查彙總。
#[derive(Debug, Default, PartialEq)]
pub(crate) struct DailyStats {
    day: Option<NaiveDate>,
    checks: u32,
    max_fsync: Option<Duration>,
    stuck_probes: u32,
    min_available_kib: Option<u64>,
    max_swap_percent: Option<f64>,
    throttled_flags: u32,
}

impl DailyStats {
    /// 記錄一次檢查；換日時先清空。
    pub(crate) fn record(&mut self, day: NaiveDate, sample: &Sample) {
        if self.day != Some(day) {
            *self = Self {
                day: Some(day),
                ..Self::default()
            };
        }
        self.checks += 1;
        if let Some(flags) = sample.throttled {
            self.throttled_flags |= flags & 0xF;
        }
        if let Some(memory) = sample.memory {
            self.min_available_kib = Some(
                self.min_available_kib
                    .map_or(memory.available_kib, |min| min.min(memory.available_kib)),
            );
            if let Some(percent) = memory.swap_used_percent() {
                self.max_swap_percent = Some(
                    self.max_swap_percent
                        .map_or(percent, |max| max.max(percent)),
                );
            }
        }
        match sample.probe {
            Probe::Done(elapsed) => {
                self.max_fsync = Some(self.max_fsync.map_or(elapsed, |max| max.max(elapsed)));
            }
            Probe::TimedOut | Probe::StillRunning | Probe::Failed(_) => self.stuck_probes += 1,
            Probe::Skipped => {}
        }
    }

    /// 當日彙總的一行文字；當天沒有檢查紀錄回 `None`。
    pub(crate) fn summary(&self, today: NaiveDate) -> Option<String> {
        if self.day != Some(today) || self.checks == 0 {
            return None;
        }
        let fsync = match (self.max_fsync, self.stuck_probes) {
            (None, 0) => "略過".to_owned(),
            (max, 0) => format!("最大 {} ms", max.unwrap_or_default().as_millis()),
            (max, stuck) => format!(
                "最大 {} ms、逾時或失敗 {stuck} 次",
                max.unwrap_or_default().as_millis()
            ),
        };
        let available = self
            .min_available_kib
            .map_or_else(|| "—".to_owned(), |kib| format!("{} MB", mb(kib)));
        let swap = self
            .max_swap_percent
            .map_or_else(|| "—".to_owned(), |percent| format!("{percent:.1}%"));
        let throttled = if self.throttled_flags == 0 {
            "無".to_owned()
        } else {
            throttled_labels(self.throttled_flags).join("、")
        };
        Some(format!(
            "主機健康彙總 {today}：檢查 {} 次｜fsync {fsync}｜MemAvailable 最低 {available}｜swap 最高 {swap}｜供電／降頻 {throttled}",
            self.checks
        ))
    }
}

/// 檢查用到的檔案路徑；測試可換成暫存檔。
#[derive(Debug, Clone)]
pub(crate) struct Paths {
    pub(crate) throttled: PathBuf,
    pub(crate) meminfo: PathBuf,
    pub(crate) probe: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            throttled: PathBuf::from(THROTTLED_PATH),
            meminfo: PathBuf::from(MEMINFO_PATH),
            probe: PathBuf::from(PROBE_PATH),
        }
    }
}

/// 告警去重狀態。只存在行程記憶體：重啟後同一個異常再告警一次是可接受的。
static GATE: Lazy<Mutex<AlertGate>> = Lazy::new(|| Mutex::new(AlertGate::default()));
/// 當日彙總。
static STATS: Lazy<Mutex<DailyStats>> = Lazy::new(|| Mutex::new(DailyStats::default()));
/// 探針是否仍在寫入；卡住的寫入結束前不再送出新的。
static PROBING: Lazy<Arc<AtomicBool>> = Lazy::new(|| Arc::new(AtomicBool::new(false)));

/// 在 `path` 寫 [`PROBE_BYTES`] 位元組並 fsync，回傳耗時。
fn write_probe(path: &Path) -> std::io::Result<Duration> {
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

/// 讀取一個小檔案（`/proc`、`/sys` 都在記憶體中，不受 SD 卡卡住影響）；不存在或讀不到回 `None`。
fn read_small(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// 讀取一次檢查的資料。
pub(crate) async fn collect(paths: &Paths, probing: &Arc<AtomicBool>, timeout: Duration) -> Sample {
    Sample {
        throttled: read_small(&paths.throttled).and_then(|text| parse_throttled(&text)),
        memory: read_small(&paths.meminfo).and_then(|text| parse_meminfo(&text)),
        probe: probe_storage(&paths.probe, probing, timeout).await,
    }
}

/// 記錄彙總並判斷這次的結果，回傳要送出的告警內容（已依冷卻時間去重）；warn 直接寫日誌。
pub(crate) fn plan_alerts(
    sample: &Sample,
    now: NaiveDateTime,
    gate: &mut AlertGate,
    stats: &mut DailyStats,
) -> Vec<String> {
    stats.record(now.date(), sample);
    let findings = evaluate(sample);
    for warning in &findings.warnings {
        tracing::warn!("主機健康檢查：{warning}");
    }
    findings
        .alerts
        .into_iter()
        .filter_map(|(kind, message)| {
            if gate.should_alert(kind, now) {
                tracing::warn!("主機健康檢查：{message}");
                Some(message)
            } else {
                tracing::info!("主機健康檢查（{ALERT_COOLDOWN_HOURS} 小時內已告警）：{message}");
                None
            }
        })
        .collect()
}

/// 排程入口（每 10 分鐘）：讀取供電、記憶體與 SD 卡狀態，異常時告警。
///
/// # Errors
///
/// 不會回傳錯誤：讀不到的項目略過，告警失敗由告警管道自行記錄。
pub async fn execute() -> Result<()> {
    let sample = collect(&Paths::default(), &PROBING, PROBE_TIMEOUT).await;
    let now = Local::now().naive_local();
    let alerts = match (GATE.lock(), STATS.lock()) {
        (Ok(mut gate), Ok(mut stats)) => plan_alerts(&sample, now, &mut gate, &mut stats),
        _ => {
            tracing::error!("主機健康檢查：狀態鎖中毒，略過本輪");
            return Ok(());
        }
    };
    for message in alerts {
        alert::send_alert("pi 主機健康異常", &message).await;
    }
    Ok(())
}

/// 排程入口（23:30）：記一行當日彙總；當天沒有檢查紀錄（非 pi）時不記。
///
/// # Errors
///
/// 不會回傳錯誤。
pub async fn summarize() -> Result<()> {
    let today = Local::now().date_naive();
    let summary = STATS.lock().ok().and_then(|stats| stats.summary(today));
    if let Some(summary) = summary {
        tracing::info!("{summary}");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
