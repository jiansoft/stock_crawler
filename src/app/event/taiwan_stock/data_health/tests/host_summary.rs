//! 本週主機健康：解析 23:30 每日彙總行、同日多行取最差值、只讀期間內的 info 檔。

use super::*;
use crate::app::event::taiwan_stock::data_health::host_summary::{
    HostDay, host_summary_line, parse_host_line, weekly_host_days,
};

const OCT30: &str = "2026-10-30 23:30:00.1 主機健康彙總 2026-10-30：檢查 49 次｜fsync 最大 14318 ms｜MemAvailable 最低 334 MB｜swap 最高 96.7%｜供電／降頻 目前 無；開機以來 曾供電不足｜部署期間 11 次（略過告警 4 項）｜資料庫：連不上 1 次（最長約 4 分鐘）、重啟 0 次、啟動於 10-29 22:55";
const NOV01: &str = "2026-11-01 23:30:00.2 主機健康彙總 2026-11-01：檢查 134 次｜fsync 最大 351 ms、逾時或失敗 1 次｜MemAvailable 最低 679 MB｜swap 最高 83.9%｜供電／降頻 目前 供電不足；開機以來 曾供電不足｜資料庫：連不上 0 次（最長約 0 分鐘）、重啟 0 次";

fn date(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).expect("日期應合法")
}

#[test]
fn parse_host_line_reads_each_field() {
    assert_eq!(
        parse_host_line(NOV01),
        Some(HostDay {
            date: Some(date(11, 1)),
            max_fsync_ms: Some(351),
            stuck_probes: 1,
            min_available_mb: Some(679),
            max_swap_percent: Some(83.9),
            throttled: true,
            db_unreachable: 0,
        })
    );
    let skipped = parse_host_line(
        "主機健康彙總 2026-11-02：檢查 3 次｜fsync 略過｜MemAvailable 最低 —｜swap 最高 —｜供電／降頻 目前 無；開機以來 —",
    )
    .expect("是彙總行");
    assert_eq!(
        (
            skipped.max_fsync_ms,
            skipped.min_available_mb,
            skipped.throttled
        ),
        (None, None, false)
    );
    assert_eq!(parse_host_line("task.done name=\"主機健康彙總\""), None);
}

/// 期間內的檔案（含輪轉檔）才讀；同一天兩行取最差值；摘要標出最差那天。
#[test]
fn weekly_host_days_merges_and_summarizes() {
    let dir =
        std::env::temp_dir().join(format!("stock_crawler_host_summary_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建立暫存目錄");
    std::fs::write(dir.join("2026-10-30_default_info.log"), OCT30).expect("寫入");
    std::fs::write(dir.join("2026-11-01_default_info.log"), NOV01).expect("寫入");
    // 同一天重啟後的另一行（輪轉檔）：MemAvailable 更低、fsync 較小。
    std::fs::write(
        dir.join("2026-11-01_default_info.23-40-00.log"),
        "主機健康彙總 2026-11-01：檢查 2 次｜fsync 最大 20 ms｜MemAvailable 最低 600 MB｜swap 最高 80.0%｜供電／降頻 目前 無；開機以來 —",
    )
    .expect("寫入");
    // 期間外與 warn 檔不讀。
    std::fs::write(
        dir.join("2026-10-20_default_info.log"),
        OCT30.replace("2026-10-30", "2026-10-20"),
    )
    .expect("寫入");
    std::fs::write(
        dir.join("2026-10-31_default_warn.log"),
        OCT30.replace("2026-10-30", "2026-10-31"),
    )
    .expect("寫入");

    let days = weekly_host_days(&dir, date(10, 27), date(11, 2));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(days.len(), 2);
    assert_eq!(days[1].min_available_mb, Some(600), "同日取最低");
    assert_eq!(days[1].max_fsync_ms, Some(351), "同日取最大");
    assert_eq!(
        host_summary_line(&days),
        "本週主機健康（2 天）：fsync 最大 14318 ms（10-30）、逾時 1 次｜MemAvailable 最低 334 MB（10-30）｜swap 最高 96.7%（10-30）｜供電／降頻 1 天｜資料庫連不上 1 次"
    );
    assert_eq!(host_summary_line(&[]), "本週主機健康：沒有每日彙總");
}
