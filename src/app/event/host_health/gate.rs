//! 告警閘門與每日彙總：同類異常的冷卻時間、開機以來位元的變化，以及 23:30 彙總用的當日統計。

use super::*;

/// 跨檢查保留的狀態：各類異常最近一次發告警的時間，以及上次讀到的開機以來位元。
#[derive(Debug, Default)]
pub(crate) struct AlertGate {
    pub(super) last_sent: HashMap<Kind, NaiveDateTime>,
    pub(super) since_boot: Option<u32>,
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
    pub(super) day: Option<NaiveDate>,
    pub(super) checks: u32,
    pub(super) max_fsync: Option<Duration>,
    pub(super) stuck_probes: u32,
    pub(super) min_available_kib: Option<u64>,
    pub(super) max_swap_percent: Option<f64>,
    pub(super) throttled_flags: u32,
    pub(super) since_boot_flags: Option<u32>,
    pub(super) deploy_checks: u32,
    pub(super) suppressed: u32,
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

/// 比對開機以來位元：行程啟動後第一次讀到記 info，之後新出現的位元記 warn。回傳記下的訊息，供測試比對。
pub(super) fn note_since_boot(sample: &Sample, gate: &mut AlertGate) -> Option<String> {
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
