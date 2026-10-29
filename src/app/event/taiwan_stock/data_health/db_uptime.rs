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
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn local(day: u32, hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 10, day, hour, minute, 0)
            .single()
            .expect("台北時間沒有夏令時間，時間唯一")
    }

    fn week_from() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 25).expect("固定日期")
    }

    /// 本週沒有重啟：只列啟動時間與連續運作時間。
    #[test]
    fn steady_server_reports_uptime_only() {
        let status = ServerStatus {
            started_at: local(19, 22, 22),
            stats_reset: Some(local(19, 22, 22)),
        };

        let line = uptime_line(Ok(status), local(31, 11, 0), week_from());

        assert_eq!(line, "資料庫：10-19 22:22 啟動，已連續運作 11 天 12 小時");
    }

    /// 本週重啟、統計在啟動時重置：判斷為非正常關機。
    #[test]
    fn restart_this_week_with_reset_stats() {
        let status = ServerStatus {
            started_at: local(29, 22, 55),
            stats_reset: Some(local(29, 22, 55)),
        };

        let line = uptime_line(Ok(status), local(31, 11, 0), week_from());

        assert_eq!(
            line,
            "資料庫：10-29 22:55 啟動，已連續運作 1 天 12 小時；本週重啟過，pg_stat_statements 統計在啟動時重置（上次多半是非正常關機）"
        );
    }

    /// 本週重啟但統計有保留，或讀不到統計。
    #[test]
    fn restart_this_week_with_kept_or_unknown_stats() {
        let kept = ServerStatus {
            started_at: local(28, 3, 0),
            stats_reset: Some(local(19, 22, 22)),
        };
        let unknown = ServerStatus {
            started_at: local(28, 3, 0),
            stats_reset: None,
        };

        let kept_line = uptime_line(Ok(kept), local(31, 11, 0), week_from());
        let unknown_line = uptime_line(Ok(unknown), local(31, 11, 0), week_from());

        assert!(
            kept_line.ends_with("本週重啟過，pg_stat_statements 統計有保留（正常關機）"),
            "{kept_line}"
        );
        assert!(
            unknown_line.ends_with("已連續運作 3 天 8 小時；本週重啟過"),
            "{unknown_line}"
        );
    }

    /// 探測失敗回一行說明。
    #[test]
    fn probe_failure_is_reported() {
        let line = uptime_line(Err("連線失敗".to_owned()), local(31, 11, 0), week_from());

        assert_eq!(line, "資料庫：讀取啟動時間失敗（連線失敗）");
    }

    /// 排程入口：連得上或連不上都回一行。
    #[tokio::test]
    async fn weekly_uptime_line_never_fails() {
        dotenvy::dotenv().ok();

        let line = weekly_uptime_line(week_from()).await;

        assert!(line.starts_with("資料庫："), "{line}");
    }
}
