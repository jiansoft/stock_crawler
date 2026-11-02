//! 本週取連線過慢：解析 sqlx 的取連線過慢 WARN、依分鐘加總、最長等待與最集中的一分鐘。

use super::*;
use crate::app::event::taiwan_stock::data_health::slow_acquire::{
    SlowAcquires, parse_acquire_line, slow_acquire_line, weekly_slow_acquires,
};

fn line(at: &str, secs: &str) -> String {
    format!(
        "{at}.884891 acquired connection, but time to acquire exceeded slow threshold acquired_after_secs={secs} slow_acquire_threshold_secs=2.0"
    )
}

#[test]
fn parse_acquire_line_reads_minute_and_seconds() {
    assert_eq!(
        parse_acquire_line(&line("2026-10-31 08:07:22", "3.727200967")),
        Some(("2026-10-31 08:07".to_owned(), 3.727200967))
    );
    let without_secs =
        "2026-10-31 08:07:22.1 acquired connection, but time to acquire exceeded slow threshold";
    assert_eq!(
        parse_acquire_line(without_secs),
        Some(("2026-10-31 08:07".to_owned(), 0.0)),
        "沒有秒數仍算一次"
    );
    assert_eq!(
        parse_acquire_line("2026-10-31 08:07:22.1 slow statement: summary=\"WITH chip\""),
        None
    );
    assert_eq!(
        parse_acquire_line("acquired connection, but time to acquire exceeded slow threshold"),
        None,
        "沒有時間戳的續行不算"
    );
}

#[test]
fn weekly_slow_acquires_reads_warn_files_in_range() {
    let dir = std::env::temp_dir().join(format!("slow_acquire_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建立暫存目錄");
    let burst: Vec<String> = ["2.1", "2.4", "3.0"]
        .iter()
        .map(|secs| line("2026-10-29 15:00:09", secs))
        .collect();
    std::fs::write(dir.join("2026-10-29_default_warn.log"), burst.join("\n")).expect("寫入");
    // 輪轉出來的舊檔也算。
    std::fs::write(
        dir.join("2026-10-31_default_warn.08-30-00.log"),
        line("2026-10-31 08:07:22", "3.7"),
    )
    .expect("寫入");
    // 期間外與 info 檔不讀。
    std::fs::write(
        dir.join("2026-10-20_default_warn.log"),
        line("2026-10-20 01:00:00", "9.9"),
    )
    .expect("寫入");
    std::fs::write(
        dir.join("2026-10-30_default_info.log"),
        line("2026-10-30 01:00:00", "9.9"),
    )
    .expect("寫入");

    let acquires = weekly_slow_acquires(
        &dir,
        NaiveDate::from_ymd_opt(2026, 10, 27).expect("日期"),
        NaiveDate::from_ymd_opt(2026, 11, 2).expect("日期"),
    );
    let missing = weekly_slow_acquires(
        &dir.join("missing"),
        NaiveDate::from_ymd_opt(2026, 10, 27).expect("日期"),
        NaiveDate::from_ymd_opt(2026, 11, 2).expect("日期"),
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        slow_acquire_line(&acquires),
        "本週取連線過慢 4 次（最長 3.7 秒，最集中在 10-29 15:00 的 3 次）"
    );
    assert_eq!(missing, SlowAcquires::default(), "目錄不存在當作 0 次");
}

#[test]
fn slow_acquire_line_skips_peak_for_scattered_minutes() {
    assert_eq!(
        slow_acquire_line(&SlowAcquires::default()),
        "本週取連線過慢 0 次"
    );
    let scattered = SlowAcquires {
        by_minute: [
            ("2026-10-28 04:00".to_owned(), 1),
            ("2026-10-30 15:00".to_owned(), 1),
        ]
        .into_iter()
        .collect(),
        max_secs: 2.25,
    };
    assert_eq!(
        slow_acquire_line(&scattered),
        "本週取連線過慢 2 次（最長 2.2 秒）",
        "每分鐘都只有一次時不列最集中的一分鐘"
    );
}
