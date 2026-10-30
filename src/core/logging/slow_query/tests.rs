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
        stats.record(at(28, 4, 0), summary, secs, Origin::Job);
    }
    assert_eq!(
        stats.summary(today(28)),
        "慢查詢 6 筆（排程 6）：WITH chip AS ( 3（最長 12.3 秒）、WITH days AS ( 1（最長 6.0 秒）、INSERT INTO estimate ( 1（最長 5.6 秒） 等 4 類"
    );
}

/// 當天啟動時註明統計起點；沒有慢查詢也印 0 筆；跨日第一筆時歸零。
#[test]
fn summary_resets_on_a_new_day_and_notes_the_start() {
    let mut stats = SlowQueryStats::new(at(28, 17, 1));
    assert_eq!(stats.summary(today(28)), "慢查詢 0 筆（自 17:01 起）");
    stats.record(at(28, 18, 0), "WITH chip AS ( …", 6.0, Origin::Job);
    assert_eq!(
        stats.summary(today(28)),
        "慢查詢 1 筆（自 17:01 起）（排程 1）：WITH chip AS ( 1（最長 6.0 秒）"
    );
    // 隔天還沒有新的慢查詢：前一天的不算。
    assert_eq!(stats.summary(today(29)), "慢查詢 0 筆");
    stats.record(at(29, 4, 0), "INSERT INTO estimate ( …", 5.1, Origin::Job);
    assert_eq!(
        stats.summary(today(29)),
        "慢查詢 1 筆（排程 1）：INSERT INTO estimate ( 1（最長 5.1 秒）"
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
        &mut slow_event(summary, 9.0),
    );
    observe(
        SQLX_TARGET,
        tracing::Level::INFO,
        &mut slow_event(summary, 9.0),
    );
    let date = Local::now().date_naive();
    assert!(!daily_summary(date).contains("p5_observe_test"));

    let mut event = slow_event(summary, 9.0);
    observe(SQLX_TARGET, tracing::Level::WARN, &mut event);
    assert!(
        !event.iter().any(|(key, _)| key == SOURCE_FIELD),
        "沒有來源標記時不加 source 欄位"
    );
    assert!(
        daily_summary(date).contains("SELECT p5_observe_test x［其他］ 1（最長 9.0 秒）"),
        "{}",
        daily_summary(date)
    );
}

/// 檔案日誌行：只認帶 `slow_threshold` 的慢查詢，取出 summary 與 elapsed_secs。
#[test]
fn parse_log_line_reads_summary_and_elapsed() {
    let line = r#"2026-10-29 04:00:10.97 slow statement: execution time exceeded alert threshold summary="INSERT INTO estimate ( …" db.statement="INSERT INTO estimate (a) VALUES (1)" rows_affected=225 elapsed="5.2s" elapsed_secs=5.209944118 slow_threshold="5s""#;
    assert_eq!(
        parse_log_line(line),
        Some((
            "INSERT INTO estimate ( …".to_string(),
            5.209944118,
            Origin::Other
        ))
    );
    assert_eq!(parse_log_line("2026-10-29 04:00:10 http.slow url=x"), None);
    assert_eq!(
        parse_log_line(r#"slow statement: x summary="SELECT 1" slow_threshold="5s""#),
        Some(("SELECT 1".to_string(), 0.0, Origin::Other))
    );
    let tagged = r#"slow statement: x summary="WITH chip AS ( …" elapsed_secs=6.2 slow_threshold="5s" source="api:/api/v1/stocks/screen/backtest/offsets""#;
    assert_eq!(
        parse_log_line(tagged),
        Some(("WITH chip AS ( …".to_string(), 6.2, Origin::Api))
    );
}

/// 沒有資料時沒有前幾類；超過列出數時附「等 N 類」。
#[test]
fn tally_lists_top_kinds() {
    let mut tally = SlowQueryTally::default();
    assert_eq!(tally.top_kinds(2), None);
    assert_eq!(tally.origins(), "");
    for (summary, secs) in [("A …", 1.0), ("B", 2.0), ("A", 3.0), ("C", 4.0)] {
        tally.add(summary, secs, Origin::Job);
    }
    assert_eq!(tally.total(), 4);
    assert_eq!(
        tally.top_kinds(2).as_deref(),
        Some("A 2（最長 3.0 秒）、C 1（最長 4.0 秒） 等 3 類")
    );
}

/// 同一個 SQL 開頭依來源分開計；非排程來源標註［API］／［其他］，依來源的筆數依排程、API、其他排序。
#[test]
fn tally_separates_origins() {
    let mut tally = SlowQueryTally::default();
    for (secs, origin) in [
        (6.2, Origin::Api),
        (5.8, Origin::Api),
        (10.2, Origin::Job),
        (5.1, Origin::Other),
    ] {
        tally.add("WITH chip AS ( …", secs, origin);
    }
    assert_eq!(tally.total(), 4);
    assert_eq!(tally.origins(), "（排程 1、API 2、其他 1）");
    assert_eq!(
        tally.top_kinds(3).as_deref(),
        Some(
            "WITH chip AS (［API］ 2（最長 6.2 秒）、WITH chip AS ( 1（最長 10.2 秒）、WITH chip AS (［其他］ 1（最長 5.1 秒）"
        )
    );
}

/// 來源標記只在 `in_job`／`in_api` 範圍內有效；`source` 欄位值依前綴分類。
#[tokio::test]
async fn origin_scopes_and_source_values() {
    assert_eq!(origin::current(), None);
    let job = in_job("收盤", async { origin::current() }).await;
    assert_eq!(job.as_deref(), Some("job:收盤"));
    let api = in_api("/api/v1/healthz", async { origin::current() }).await;
    assert_eq!(api.as_deref(), Some("api:/api/v1/healthz"));
    assert_eq!(origin::current(), None);

    assert_eq!(Origin::from_source(job.as_deref()), Origin::Job);
    assert_eq!(Origin::from_source(api.as_deref()), Origin::Api);
    assert_eq!(Origin::from_source(Some("x:1")), Origin::Other);
    assert_eq!(Origin::from_source(None), Origin::Other);
}

/// 在排程範圍內的慢查詢事件：附上 `source` 欄位並記為排程。
#[tokio::test]
async fn observe_tags_events_with_the_current_origin() {
    init();
    let summary = "SELECT o8_origin_test x …";
    let mut event = slow_event(summary, 7.0);
    in_job("O8 測試", async {
        observe(SQLX_TARGET, tracing::Level::WARN, &mut event)
    })
    .await;
    assert!(
        event
            .iter()
            .any(|(key, value)| key == SOURCE_FIELD && value == "job:O8 測試"),
        "{event:?}"
    );
    let date = Local::now().date_naive();
    assert!(
        daily_summary(date).contains("SELECT o8_origin_test x 1（最長 7.0 秒）"),
        "{}",
        daily_summary(date)
    );
}
