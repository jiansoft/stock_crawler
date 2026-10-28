//! # 主機健康檢查
//!
//! 正式站跑在 Raspberry Pi 上。2026-10-27 部署時 MCP 的 SIGTERM 晚了 31 秒才被處理，kernel 同時記錄
//! `mmc_rescan blocked for more than 122 seconds`（10-24 也有一次），`get_throttled` 顯示開機以來曾經
//! 供電不足並降頻。這些狀況不會讓任何排程報錯，只會讓部署、日誌寫入與資料庫連線莫名變慢，因此每 10 分鐘
//! 讀一次：
//!
//! - **供電與降頻**：sysfs `get_throttled` 的「目前」位元（bit 0 供電不足、bit 2 降頻）告警；
//!   `vcgencmd get_throttled` 的「開機以來曾經發生」位元（bit 16 起）新出現時記 warn。sysfs 只給目前位元，
//!   欠壓常只有幾秒（10-28 09:52 那次 4 秒），10 分鐘一次的檢查幾乎看不到，開機以來位元才留得下證據。
//! - **記憶體**：`/proc/meminfo` 的 `MemAvailable` 與 swap 使用率。
//! - **SD 卡**：在 `log/` 寫 4 KB 並 `fsync` 的耗時；I/O 卡住時第一個受害的就是日誌與部署。
//!
//! 部署本身會讓 swap 用滿、fsync 變慢（10-28 08:10 部署 stock_go web 時 fsync 22.6 秒）。部署腳本在
//! `/dev/shm/stock-deploying/` 為每個單元寫一個標記檔，最後異動後 [`DEPLOY_GRACE`] 內視為部署期間：
//! 記憶體與 SD 卡的超標只記 info、不發告警（供電照常告警）；標記沒寫入結束又超過時間視為殘留，照常告警。
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
    time::{Duration, Instant, SystemTime},
};

use anyhow::Result;
use chrono::{Local, NaiveDate, NaiveDateTime};
use once_cell::sync::Lazy;

use crate::core::alert;

/// 樹莓派韌體回報供電與降頻狀態的 sysfs 檔案（十六進位，不帶 `0x`）；只有「目前」位元。
const THROTTLED_PATH: &str = "/sys/devices/platform/soc/soc:firmware/get_throttled";
/// 韌體指令；`get_throttled` 輸出 `throttled=0x…`，含「開機以來曾經發生」的位元。
const VCGENCMD_PATH: &str = "/usr/bin/vcgencmd";
/// `vcgencmd` 最多等多久。
const VCGENCMD_TIMEOUT: Duration = Duration::from_secs(5);
/// 系統記憶體資訊。
const MEMINFO_PATH: &str = "/proc/meminfo";
/// SD 卡探針寫入的檔案；與日誌同一個目錄，量到的就是日誌實際寫入的延遲。
const PROBE_PATH: &str = "log/.host_health_probe";
/// 探針每次寫入的位元組數。
const PROBE_BYTES: usize = 4096;
/// 部署腳本寫標記的目錄（tmpfs，不受 SD 卡卡住影響，重開機清空）。
///
/// 每個部署單元一個檔案：開始時寫 `unit=`／`started=`，結束時補一行 `finished=`。
const DEPLOY_MARKER_DIR: &str = "/dev/shm/stock-deploying";
/// 標記最後異動後多久內視為部署期間；涵蓋部署結束後 swap 換回與 I/O 回穩的時間。
const DEPLOY_GRACE: Duration = Duration::from_secs(30 * 60);

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
/// 「開機以來曾經發生」位元的位移：bit 16～19 依序對應 bit 0～3。
const SINCE_BOOT_SHIFT: u32 = 16;

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

/// 最近一次異動的部署標記。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeployMarker {
    /// 部署單元（標記內容的 `unit=`，沒有就用檔名）。
    pub(crate) unit: String,
    /// 距標記最後異動多久。
    pub(crate) age: Duration,
    /// 部署腳本是否已寫入結束。
    pub(crate) finished: bool,
}

impl DeployMarker {
    /// 是否仍在部署期間（含結束後的緩衝）。
    fn active(&self) -> bool {
        self.age <= DEPLOY_GRACE
    }

    /// 沒寫入結束又超過時間：部署腳本中途被砍掉留下的殘留標記。
    fn stale(&self) -> bool {
        !self.finished && !self.active()
    }
}

/// 一次檢查讀到的資料；讀不到的項目為 `None`。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Sample {
    /// sysfs `get_throttled`（只有目前位元）。
    pub(crate) throttled: Option<u32>,
    /// `vcgencmd get_throttled`（含開機以來位元）。
    pub(crate) since_boot: Option<u32>,
    pub(crate) memory: Option<MemInfo>,
    pub(crate) probe: Probe,
    /// 最近一次異動的部署標記；沒有標記為 `None`。
    pub(crate) deploy: Option<DeployMarker>,
}

/// 一次檢查的判斷結果。
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Findings {
    /// 要發告警的異常。
    pub(crate) alerts: Vec<(Kind, String)>,
    /// 只記 warn 的異常。
    pub(crate) warnings: Vec<String>,
    /// 部署期間略過告警、只記 info 的異常。
    pub(crate) suppressed: Vec<String>,
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

/// 解析 `vcgencmd get_throttled` 的輸出（`throttled=0x70000`）；格式不符回 `None`。
pub(crate) fn parse_vcgencmd_throttled(text: &str) -> Option<u32> {
    parse_throttled(text.trim().strip_prefix("throttled=")?)
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

/// 解析部署標記內容；`unit=` 缺少時用 `fallback_unit`（檔名）。
pub(crate) fn parse_deploy_marker(text: &str, fallback_unit: &str, age: Duration) -> DeployMarker {
    let unit = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("unit="))
        .filter(|unit| !unit.is_empty())
        .unwrap_or(fallback_unit)
        .to_owned();
    DeployMarker {
        unit,
        age,
        finished: text
            .lines()
            .any(|line| line.trim().starts_with("finished=")),
    }
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

/// 開機以來位元（已右移到 bit 0～3）的中文說明，例如「曾供電不足、曾降頻」；沒有任何位元時為「無」。
fn since_boot_text(flags: u32) -> String {
    if flags == 0 {
        return "無".to_owned();
    }
    throttled_labels(flags)
        .into_iter()
        .map(|label| format!("曾{label}"))
        .collect::<Vec<_>>()
        .join("、")
}

/// 取出 `vcgencmd` 值中的開機以來位元，右移到 bit 0～3。
fn since_boot_flags(value: u32) -> u32 {
    (value >> SINCE_BOOT_SHIFT) & 0xF
}

/// KiB 換成 MB（取整）。
fn mb(kib: u64) -> u64 {
    kib / 1024
}

/// 秒數文字（一位小數）。
fn seconds(duration: Duration) -> String {
    format!("{:.1}", duration.as_secs_f64())
}

/// 判斷一次檢查的結果：哪些要告警、哪些只記 warn、哪些因部署期間略過。
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

    match &sample.deploy {
        // 部署期間：記憶體與 SD 卡的超標是部署本身造成的，改記 info；供電與部署無關，照常告警。
        Some(marker) if marker.active() => {
            let (power, deploy_related): (Vec<_>, Vec<_>) = std::mem::take(&mut findings.alerts)
                .into_iter()
                .partition(|(kind, _)| *kind == Kind::Power);
            findings.alerts = power;
            findings.suppressed = deploy_related
                .into_iter()
                .map(|(_, message)| message)
                .chain(std::mem::take(&mut findings.warnings))
                .collect();
        }
        Some(marker)
            if marker.stale() && !(findings.alerts.is_empty() && findings.warnings.is_empty()) =>
        {
            findings.warnings.push(format!(
                "部署標記 {} 已 {} 分鐘沒有寫入結束，視為殘留，照常告警",
                marker.unit,
                marker.age.as_secs() / 60
            ));
        }
        _ => {}
    }

    findings
}

/// 跨檢查保留的狀態：各類異常最近一次發告警的時間，以及上次讀到的開機以來位元。
#[derive(Debug, Default)]
pub(crate) struct AlertGate {
    last_sent: HashMap<Kind, NaiveDateTime>,
    since_boot: Option<u32>,
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

    /// 記下這次的開機以來位元，回傳上次之後新出現的位元；行程啟動後第一次讀到時回傳 `None`。
    pub(crate) fn new_since_boot(&mut self, flags: u32) -> Option<u32> {
        self.since_boot
            .replace(flags)
            .map(|previous| flags & !previous)
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
    since_boot_flags: Option<u32>,
    deploy_checks: u32,
    suppressed: u32,
}

impl DailyStats {
    /// 記錄一次檢查與這次因部署略過的異常數；換日時先清空。
    pub(crate) fn record(&mut self, day: NaiveDate, sample: &Sample, suppressed: usize) {
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
        if let Some(value) = sample.since_boot {
            self.since_boot_flags =
                Some(self.since_boot_flags.unwrap_or(0) | since_boot_flags(value));
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
        if sample.deploy.as_ref().is_some_and(DeployMarker::active) {
            self.deploy_checks += 1;
        }
        self.suppressed += u32::try_from(suppressed).unwrap_or(u32::MAX);
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
        let since_boot = self
            .since_boot_flags
            .map_or_else(|| "—".to_owned(), since_boot_text);
        let deploy = if self.deploy_checks == 0 {
            String::new()
        } else {
            format!(
                "｜部署期間 {} 次（略過告警 {} 項）",
                self.deploy_checks, self.suppressed
            )
        };
        Some(format!(
            "主機健康彙總 {today}：檢查 {} 次｜fsync {fsync}｜MemAvailable 最低 {available}｜swap 最高 {swap}｜供電／降頻 目前 {throttled}；開機以來 {since_boot}{deploy}",
            self.checks
        ))
    }
}

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

/// 告警去重與開機以來位元的狀態。只存在行程記憶體：重啟後同一個異常再告警一次是可接受的。
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

/// 讀取一個小檔案（`/proc`、`/sys`、`/dev/shm` 都在記憶體中，不受 SD 卡卡住影響）；不存在或讀不到回 `None`。
fn read_small(path: &Path) -> Option<String> {
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

/// 讀取部署標記目錄，回傳最近異動的一個；目錄不存在或沒有標記回 `None`。
pub(crate) fn read_deploy_marker(dir: &Path, now: SystemTime) -> Option<DeployMarker> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            let age = now
                .duration_since(metadata.modified().ok()?)
                .unwrap_or_default();
            let text = read_small(&entry.path()).unwrap_or_default();
            Some(parse_deploy_marker(
                &text,
                &entry.file_name().to_string_lossy(),
                age,
            ))
        })
        .min_by_key(|marker| marker.age)
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

/// 比對開機以來位元：行程啟動後第一次讀到記 info，之後新出現的位元記 warn。回傳記下的訊息，供測試比對。
fn note_since_boot(sample: &Sample, gate: &mut AlertGate) -> Option<String> {
    let value = sample.since_boot?;
    let flags = since_boot_flags(value);
    match gate.new_since_boot(flags) {
        Some(new) if new != 0 => {
            let message = format!(
                "開機以來新出現{}（vcgencmd get_throttled=0x{value:x}）；發生在上次檢查之後，確切時間請查 `dmesg`",
                since_boot_text(new)
            );
            tracing::warn!("主機健康檢查：{message}");
            Some(message)
        }
        None if flags != 0 => {
            let message = format!(
                "開機以來{}（vcgencmd get_throttled=0x{value:x}）",
                since_boot_text(flags)
            );
            tracing::info!("主機健康檢查：{message}");
            Some(message)
        }
        _ => None,
    }
}

/// 記錄彙總並判斷這次的結果，回傳要送出的告警內容（已依冷卻時間去重）；warn 與部署期間略過的項目直接寫日誌。
pub(crate) fn plan_alerts(
    sample: &Sample,
    now: NaiveDateTime,
    gate: &mut AlertGate,
    stats: &mut DailyStats,
) -> Vec<String> {
    let findings = evaluate(sample);
    stats.record(now.date(), sample, findings.suppressed.len());
    note_since_boot(sample, gate);
    for warning in &findings.warnings {
        tracing::warn!("主機健康檢查：{warning}");
    }
    if let Some(marker) = &sample.deploy {
        for message in &findings.suppressed {
            tracing::info!(
                "主機健康檢查（{} 部署期間，略過告警）：{message}",
                marker.unit
            );
        }
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
