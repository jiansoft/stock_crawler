//! 本週慢查詢：只讀期間內的 warn 檔（含輪轉檔），依 SQL 開頭歸類並附在週報最後。

use super::*;
use crate::core::logging::slow_query::{Origin, SlowQueryTally};

const SLOW: &str = r#"slow statement: execution time exceeded alert threshold summary="INSERT INTO estimate ( …" db.statement="INSERT INTO estimate ..." rows_affected=225 rows_returned=0 elapsed="5.2s" elapsed_secs=5.25 slow_threshold="5s""#;
const CHIP: &str = r#"slow statement: execution time exceeded alert threshold summary="WITH chip AS ( …" db.statement="WITH chip AS ..." rows_affected=0 rows_returned=10 elapsed="12.3s" elapsed_secs=12.34 slow_threshold="5s" source="job:收盤""#;

/// 測試專用的暫存目錄，結束時刪除。
struct LogDir(std::path::PathBuf);

impl LogDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("stock_crawler_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建立暫存目錄");
        Self(dir)
    }

    fn write(&self, file: &str, lines: &[&str]) {
        let body: Vec<String> = lines
            .iter()
            .map(|line| format!("2026-10-08 04:00:10.97 {line}"))
            .collect();
        std::fs::write(self.0.join(file), body.join("\n")).expect("寫入日誌");
    }
}

impl Drop for LogDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn weekly_slow_queries_reads_warn_files_in_the_period_only() {
    let dir = LogDir::new("weekly_slow_queries");
    dir.write(
        "2026-10-08_default_warn.log",
        &[SLOW, "http.slow url=x", CHIP],
    );
    // 輪轉出來的舊檔也算。
    dir.write("2026-10-09_default_warn.04-10-00.log", &[SLOW]);
    // 期間外、非 warn 檔、檔名不是日期開頭：不算。
    dir.write("2026-10-03_default_warn.log", &[CHIP]);
    dir.write("2026-10-08_default_info.log", &[CHIP]);
    dir.write("notes_default_warn.log", &[CHIP]);

    let tally = weekly_slow_queries(&dir.0, day(4), day(10));
    assert_eq!(tally.total(), 3);
    assert_eq!(
        slow_query_line(&tally),
        "本週慢查詢 3 筆（排程 1、其他 2）：INSERT INTO estimate (［其他］ 2（最長 5.2 秒）、WITH chip AS ( 1（最長 12.3 秒）"
    );
    // 目錄不存在時當作 0 筆。
    let missing = weekly_slow_queries(&dir.0.join("missing"), day(4), day(10));
    assert_eq!(slow_query_line(&missing), "本週慢查詢 0 筆");
}

#[test]
fn slow_queries_are_appended_to_the_report() {
    let mut tally = SlowQueryTally::default();
    tally.add("WITH chip AS ( …", 6.0, Origin::Api);
    let message = with_slow_queries(report(day(4), day(10), &[]), &tally);
    assert!(
        // MarkdownV2 的保留字元（括號、小數點）要跳脫。
        message.ends_with(concat!(
            "\n",
            r"本週慢查詢 1 筆（API 1）：WITH chip AS \(［API］ 1（最長 6\.0 秒）"
        )),
        "{message}"
    );
}
