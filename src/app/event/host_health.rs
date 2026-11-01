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

use crate::core::{alert, logging};

mod deploy;
mod gate;
mod parse;
mod sample;

// 解析、部署標記、告警閘門與取樣搬到子模組；型別、判斷規則與排程入口留在這裡。
use self::{deploy::*, gate::*, parse::*, sample::*};

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
/// swap 使用率高於此值（%），且 `MemAvailable` 低於 [`SWAP_PRESSURE_AVAILABLE_KIB`] 時告警。
///
/// 10-27 實測已用 79%，主要是閒置的 `mysqld` 被換出，本身不代表記憶體不足；用滿才是警訊。
const MAX_SWAP_USED_PERCENT: f64 = 95.0;
/// swap 用滿時，`MemAvailable` 低於此值（KiB）才算記憶體有壓力。
///
/// 11-02 05:30 swap 95.3% 告警時 `MemAvailable` 還有 699 MB：換出的是閒置的 mysqld、php-fpm，
/// 不是記憶體不夠用。可用記憶體平常約 700 MB，掉到 400 MB 以下又同時 swap 用滿才是真的吃緊。
const SWAP_PRESSURE_AVAILABLE_KIB: u64 = 400 * 1024;
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
        // swap 用滿但可用記憶體充足時不告警（換出的是閒置程序），23:30 彙總仍列 swap 最高值。
        if let Some(percent) = memory.swap_used_percent()
            && percent > MAX_SWAP_USED_PERCENT
            && memory.available_kib < SWAP_PRESSURE_AVAILABLE_KIB
        {
            problems.push(format!(
                "swap 已用 {percent:.1}%（{}／{} MB，門檻 {MAX_SWAP_USED_PERCENT:.0}%；可用記憶體 {} MB，低於 {} MB）",
                mb(memory.swap_total_kib.saturating_sub(memory.swap_free_kib)),
                mb(memory.swap_total_kib),
                mb(memory.available_kib),
                mb(SWAP_PRESSURE_AVAILABLE_KIB)
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

/// 告警去重與開機以來位元的狀態。只存在行程記憶體：重啟後同一個異常再告警一次是可接受的。
static GATE: Lazy<Mutex<AlertGate>> = Lazy::new(|| Mutex::new(AlertGate::default()));
/// 當日彙總。
static STATS: Lazy<Mutex<DailyStats>> = Lazy::new(|| Mutex::new(DailyStats::default()));
/// 探針是否仍在寫入；卡住的寫入結束前不再送出新的。
static PROBING: Lazy<Arc<AtomicBool>> = Lazy::new(|| Arc::new(AtomicBool::new(false)));

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

/// 排程入口（23:30）：記一行當日彙總，後面附當日資料庫可用性（[`super::db_health::daily_summary`]）
/// 與慢查詢（[`logging::slow_query::daily_summary`]）；
/// 當天沒有主機檢查紀錄（非 pi）時只記慢查詢。
///
/// # Errors
///
/// 不會回傳錯誤。
pub async fn summarize() -> Result<()> {
    let today = Local::now().date_naive();
    let summary = STATS.lock().ok().and_then(|stats| stats.summary(today));
    let slow = logging::slow_query::daily_summary(today);
    let slow = match super::db_health::daily_summary(today) {
        Some(database) => format!("{database}｜{slow}"),
        None => slow,
    };
    match summary {
        Some(summary) => tracing::info!("{summary}｜{slow}"),
        None => tracing::info!("慢查詢彙總 {today}：{slow}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests;
