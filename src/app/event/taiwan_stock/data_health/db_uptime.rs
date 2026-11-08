//! 健康週報的資料庫可用性一行：啟動時間、連續運作多久，本週重啟過時附上 `pg_stat_statements`
//! 是否在啟動時重置。
//!
//! 2026-10-29 正式庫非正常停止後重啟，統計全部歸零；週報的資料庫端耗時雖然會寫「統計重置起」，
//! 但看不出是哪一天、為什麼重置。這一行讓週報直接說明。

use std::time::Duration;

use chrono::{DateTime, Local, NaiveDate};

use crate::infra::database::server_status::{self, ServerStatus};

/// 探測最多等多久。
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// 探測資料庫並組出週報的一行（未跳脫）；探測失敗也回一行說明，不讓週報失敗。
pub(super) async fn weekly_uptime_line(week_from: NaiveDate) -> String {
    let status = server_status::probe(PROBE_TIMEOUT)
        .await
        .map_err(|why| format!("{why:#}"));
    uptime_line(status, Local::now(), week_from)
}

/// `week_from`（含）之後啟動的視為本週重啟過。
pub(super) fn uptime_line(
    status: Result<ServerStatus, String>,
    now: DateTime<Local>,
    week_from: NaiveDate,
) -> String {
    let status = match status {
        Ok(status) => status,
        Err(why) => return format!("資料庫：讀取啟動時間失敗（{why}）"),
    };
    let uptime = now - status.started_at;
    let mut line = format!(
        "資料庫：{} 啟動，已連續運作 {} 天 {} 小時",
        status.started_at.format("%m-%d %H:%M"),
        uptime.num_days(),
        uptime.num_hours() % 24
    );
    if status.started_at.date_naive() >= week_from {
        line.push_str(match status.stats_reset_at_start() {
            Some(true) => {
                "；本週重啟過，pg_stat_statements 統計在啟動時重置（上次多半是非正常關機）"
            }
            Some(false) => "；本週重啟過，pg_stat_statements 統計有保留（正常關機）",
            None => "；本週重啟過",
        });
    }
    line
}

#[cfg(test)]
mod tests;
