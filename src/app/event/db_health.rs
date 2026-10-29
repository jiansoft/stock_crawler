//! # 資料庫停機與重啟監控
//!
//! 正式庫跑在另一台主機，不是 Windows 服務。2026-10-29 22:49～22:55 它非正常停止，這段時間 pi
//! 剛好沒有排程，沒有任何告警，是使用者自己發現的；重啟後 `pg_stat_statements` 與表統計全部歸零，
//! 慢查詢彙總與週報的資料庫端耗時也跟著換了起算點。因此每 5 分鐘探測一次：
//!
//! - **連不上**：連續 [`DOWN_ALERT_AFTER`] 次失敗發告警，之後 [`DOWN_ALERT_COOLDOWN_MINUTES`] 分鐘
//!   內不重發；告警過才在恢復時通知，只失敗一次的網路抖動不通知。
//! - **重啟**：`pg_postmaster_start_time()` 與上次看到的不同時通知，附上 `pg_stat_statements` 是否在
//!   啟動時重置——正常關機會保存統計，重置代表當機或 immediate stop。服務剛啟動時只記下啟動時間。
//!
//! 探測用獨立的短連線（[`server_status`]），不經連線池。狀態只存在行程記憶體。

use std::{sync::Mutex, time::Duration as StdDuration};

use anyhow::Result;
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime};
use once_cell::sync::Lazy;

use crate::{
    core::alert,
    infra::database::server_status::{self, ServerStatus},
};

/// 每次探測最多等多久（連線＋查詢）。
const PROBE_TIMEOUT: StdDuration = StdDuration::from_secs(5);
/// 連續失敗幾次才告警（每 5 分鐘一次，2 次約 5～10 分鐘）。
const DOWN_ALERT_AFTER: u32 = 2;
/// 持續連不上時，告警的最短間隔（分鐘）。
const DOWN_ALERT_COOLDOWN_MINUTES: i64 = 60;
/// 統計重置時間與啟動時間相差在這個範圍內，視為啟動時重置（實測差 20 ms）。
const RESET_AT_START_TOLERANCE_SECONDS: i64 = 60;

/// 要發出的通知。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notice {
    pub(crate) title: &'static str,
    pub(crate) message: String,
}

/// 探測結果的累積狀態。
#[derive(Debug, Default)]
pub(crate) struct Watch {
    /// 上次看到的伺服器啟動時間；服務剛啟動、還沒連上過時為 `None`。
    started_at: Option<DateTime<Local>>,
    /// 目前連續失敗次數。
    failures: u32,
    /// 這一段連不上的第一次失敗時間。
    down_since: Option<NaiveDateTime>,
    /// 這一段連不上最後一次發告警的時間；沒告警過為 `None`。
    last_down_alert: Option<NaiveDateTime>,
    /// 當日彙總。
    daily: Daily,
}

/// 當日彙總（23:30 記一行）。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Daily {
    date: Option<NaiveDate>,
    failures: u32,
    longest_down_minutes: i64,
    restarts: u32,
}

impl Watch {
    /// 記錄一次探測結果，回傳要發的通知。
    pub(crate) fn observe(
        &mut self,
        probe: std::result::Result<ServerStatus, String>,
        now: NaiveDateTime,
    ) -> Option<Notice> {
        if self.daily.date != Some(now.date()) {
            self.daily = Daily {
                date: Some(now.date()),
                ..Daily::default()
            };
        }
        match probe {
            Err(why) => self.on_failure(&why, now),
            Ok(status) => self.on_success(status, now),
        }
    }

    fn on_failure(&mut self, why: &str, now: NaiveDateTime) -> Option<Notice> {
        self.failures += 1;
        self.daily.failures += 1;
        let since = *self.down_since.get_or_insert(now);
        if self.failures < DOWN_ALERT_AFTER {
            return None;
        }
        let cooled = self
            .last_down_alert
            .is_none_or(|last| now - last >= Duration::minutes(DOWN_ALERT_COOLDOWN_MINUTES));
        if !cooled {
            return None;
        }
        self.last_down_alert = Some(now);
        Some(Notice {
            title: "資料庫連不上",
            message: format!(
                "自 {} 起連續 {} 次連不上（每 5 分鐘探測一次）：{why}；pi 上所有排程與 API 都會失敗，請檢查資料庫主機",
                since.format("%m-%d %H:%M"),
                self.failures
            ),
        })
    }

    fn on_success(&mut self, status: ServerStatus, now: NaiveDateTime) -> Option<Notice> {
        let recovered = self.recover(now);
        let restarted = match self.started_at.replace(status.started_at) {
            Some(previous) if previous != status.started_at => {
                self.daily.restarts += 1;
                Some(restart_message(previous, &status))
            }
            _ => None,
        };
        match (recovered, restarted) {
            (None, None) => None,
            (Some(recovered), None) => Some(Notice {
                title: "資料庫已恢復",
                message: recovered,
            }),
            (recovered, Some(restarted)) => Some(Notice {
                title: "資料庫重啟過",
                message: match recovered {
                    Some(recovered) => format!("{recovered}；{restarted}"),
                    None => restarted,
                },
            }),
        }
    }

    /// 結束一段連不上；告警過才回傳恢復訊息。
    fn recover(&mut self, now: NaiveDateTime) -> Option<String> {
        self.failures = 0;
        let since = self.down_since.take()?;
        let minutes = (now - since).num_minutes();
        self.daily.longest_down_minutes = self.daily.longest_down_minutes.max(minutes);
        self.last_down_alert.take()?;
        Some(format!(
            "已恢復連線；第一次探測失敗在 {}，約 {minutes} 分鐘連不上",
            since.format("%m-%d %H:%M")
        ))
    }

    /// 當日彙總；`today` 沒有任何探測紀錄時為 `None`。
    pub(crate) fn summary(&self, today: NaiveDate) -> Option<String> {
        if self.daily.date != Some(today) {
            return None;
        }
        let started = self.started_at.map_or_else(
            || "啟動時間未知".to_string(),
            |at| format!("啟動於 {}", at.format("%m-%d %H:%M")),
        );
        Some(format!(
            "資料庫：連不上 {} 次（最長約 {} 分鐘）、重啟 {} 次、{started}",
            self.daily.failures, self.daily.longest_down_minutes, self.daily.restarts
        ))
    }
}

/// 重啟通知內容：新舊啟動時間與統計是否在啟動時重置。
fn restart_message(previous: DateTime<Local>, status: &ServerStatus) -> String {
    let stats = match status.stats_reset {
        Some(reset)
            if (reset - status.started_at).num_seconds().abs()
                <= RESET_AT_START_TOLERANCE_SECONDS =>
        {
            "pg_stat_statements 統計在啟動時重置，上次多半是非正常關機（當機或 immediate stop）；慢查詢彙總與週報的資料庫端耗時從這次啟動起算"
        }
        Some(_) => "pg_stat_statements 統計有保留，上次是正常關機",
        None => "讀不到 pg_stat_statements，無法判斷是否正常關機",
    };
    format!(
        "資料庫在 {} 重新啟動（上次啟動 {}）；{stats}",
        status.started_at.format("%m-%d %H:%M:%S"),
        previous.format("%m-%d %H:%M")
    )
}

/// 探測狀態。只存在行程記憶體：服務重啟後第一次探測只記下啟動時間。
static WATCH: Lazy<Mutex<Watch>> = Lazy::new(|| Mutex::new(Watch::default()));

/// 排程入口（每 5 分鐘）：探測資料庫，連不上或重啟過時告警。
///
/// # Errors
///
/// 不會回傳錯誤：探測失敗本身就是要回報的狀況，告警失敗由告警管道自行記錄。
pub async fn execute() -> Result<()> {
    let probe = server_status::probe(PROBE_TIMEOUT)
        .await
        .map_err(|why| format!("{why:#}"));
    if let Err(why) = &probe {
        tracing::warn!("資料庫健康檢查：連不上：{why}");
    }
    let now = Local::now().naive_local();
    let notice = match WATCH.lock() {
        Ok(mut watch) => watch.observe(probe, now),
        Err(_) => {
            tracing::error!("資料庫健康檢查：狀態鎖中毒，略過本輪");
            return Ok(());
        }
    };
    if let Some(notice) = notice {
        tracing::warn!("{}：{}", notice.title, notice.message);
        alert::send_alert(notice.title, &notice.message).await;
    }
    Ok(())
}

/// 當日資料庫可用性彙總，附在 23:30 主機健康彙總後面；當天沒有探測紀錄時為 `None`。
pub fn daily_summary(today: NaiveDate) -> Option<String> {
    WATCH.lock().ok().and_then(|watch| watch.summary(today))
}

#[cfg(test)]
mod tests;
