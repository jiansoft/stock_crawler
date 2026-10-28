//! 慢查詢當日統計的測試：歸類、跨日歸零、彙總文字與事件欄位判斷。

use super::*;

fn at(day: u32, hour: u32, minute: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 10, day)
        .and_then(|date| date.and_hms_opt(hour, minute, 0))
        .expect("時間應合法")
}

fn today(day: u32) -> NaiveDate {
    at(day, 0, 0).date()
}

fn slow_event(summary: &str, secs: f64) -> Vec<(String, serde_json::Value)> {
    vec![
        ("summary".to_string(), serde_json::Value::from(summary)),
        ("db.statement".to_string(), serde_json::Value::from("…")),
        ("elapsed_secs".to_string(), serde_json::Value::from(secs)),
        ("slow_threshold".to_string(), serde_json::Value::from("5s")),
    ]
}

/// 依 SQL 開頭歸類，列出筆數最多的前三類與最長耗時；更多類別只計入總數。
#[test]
fn summary_lists_top_kinds_with_longest_elapsed() {
    let mut stats = SlowQueryStats::new(at(27, 17, 1));
    for (summary, secs) in [
        ("WITH chip AS ( …", 5.2),
        ("WITH chip AS ( …", 12.34),
        ("INSERT INTO estimate ( …", 5.6),
        ("WITH days AS ( …", 6.0),
        ("WITH chip AS ( …", 7.0),
        ("SELECT 1", 1.2),
    ] {
        stats.record(at(28, 4, 0), summary, secs);
    }
    assert_eq!(
        stats.summary(today(28)),
        "慢查詢 6 筆：WITH chip AS ( 3（最長 12.3 秒）、WITH days AS ( 1（最長 6.0 秒）、INSERT INTO estimate ( 1（最長 5.6 秒） 等 4 類"
    );
}

/// 當天啟動時註明統計起點；沒有慢查詢也印 0 筆；跨日第一筆時歸零。
#[test]
fn summary_resets_on_a_new_day_and_notes_the_start() {
    let mut stats = SlowQueryStats::new(at(28, 17, 1));
    assert_eq!(stats.summary(today(28)), "慢查詢 0 筆（自 17:01 起）");
    stats.record(at(28, 18, 0), "WITH chip AS ( …", 6.0);
    assert_eq!(
        stats.summary(today(28)),
        "慢查詢 1 筆（自 17:01 起）：WITH chip AS ( 1（最長 6.0 秒）"
    );
    // 隔天還沒有新的慢查詢：前一天的不算。
    assert_eq!(stats.summary(today(29)), "慢查詢 0 筆");
    stats.record(at(29, 4, 0), "INSERT INTO estimate ( …", 5.1);
    assert_eq!(
        stats.summary(today(29)),
        "慢查詢 1 筆：INSERT INTO estimate ( 1（最長 5.1 秒）"
    );
}

/// 只有帶 `slow_threshold` 的事件才是慢查詢；缺 `elapsed_secs` 時以 0 計。
#[test]
fn only_events_with_slow_threshold_are_slow_statements() {
    let event = slow_event("WITH chip AS ( …", 5.5);
    assert_eq!(
        slow_statement_fields(&event),
        Some(("WITH chip AS ( …", 5.5))
    );

    let ordinary: Vec<_> = event
        .iter()
        .filter(|(key, _)| key != SLOW_THRESHOLD_FIELD)
        .cloned()
        .collect();
    assert_eq!(slow_statement_fields(&ordinary), None);

    let no_elapsed: Vec<_> = event
        .iter()
        .filter(|(key, _)| key != "elapsed_secs")
        .cloned()
        .collect();
    assert_eq!(
        slow_statement_fields(&no_elapsed),
        Some(("WITH chip AS ( …", 0.0))
    );
}

/// 只記 sqlx 的 WARN 慢查詢事件；其他 target 或等級略過。
#[test]
fn observe_records_sqlx_warnings_only() {
    init();
    let summary = "SELECT p5_observe_test x …";
    observe(
        "stock_crawler::app",
        tracing::Level::WARN,
        &slow_event(summary, 9.0),
    );
    observe(SQLX_TARGET, tracing::Level::INFO, &slow_event(summary, 9.0));
    let date = Local::now().date_naive();
    assert!(!daily_summary(date).contains("p5_observe_test"));

    observe(SQLX_TARGET, tracing::Level::WARN, &slow_event(summary, 9.0));
    assert!(
        daily_summary(date).contains("SELECT p5_observe_test x 1（最長 9.0 秒）"),
        "{}",
        daily_summary(date)
    );
}
