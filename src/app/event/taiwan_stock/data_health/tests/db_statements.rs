//! 資料庫端耗時：以上次週報的快照算差額，統計重置過或沒有快照時改列累計。

use chrono::{DateTime, Local, TimeZone};

use super::super::db_statements::{
    SavedStatements, TopStatement, load, save, statement_line, top_statements,
};
use super::*;
use crate::{domain::health::StatementStat, infra::nosql::redis::CLIENT};

fn at(day: u32, hour: u32) -> DateTime<Local> {
    Local
        .with_ymd_and_hms(2026, 10, day, hour, 0, 0)
        .single()
        .expect("時間應合法")
}

fn stat(queryid: i64, calls: i64, total_ms: f64, query: &str) -> StatementStat {
    StatementStat {
        queryid,
        calls,
        total_ms,
        query: query.to_string(),
    }
}

fn stats(reset: DateTime<Local>, rows: Vec<StatementStat>) -> StatementStats {
    StatementStats {
        stats_reset: reset,
        rows,
    }
}

fn week_ago() -> SavedStatements {
    SavedStatements::new(
        &stats(
            at(19, 22),
            vec![
                stat(1, 100, 100_000.0, "WITH chip AS (…)"),
                stat(2, 10, 50_000.0, "SELECT 2"),
                stat(3, 50, 900.0, "SELECT 3"),
            ],
        ),
        at(24, 11),
    )
}

fn now() -> StatementStats {
    stats(
        at(19, 22),
        vec![
            // 本週 20 次、40 秒。
            stat(1, 120, 140_000.0, "WITH chip AS (…)"),
            // 本週沒跑。
            stat(2, 10, 50_000.0, "SELECT 2"),
            // 被淘汰後重新計數：比上次小，不列。
            stat(3, 5, 100.0, "SELECT 3"),
            // 新語句：整段都算本週。
            stat(4, 2, 60_000.0, "SELECT   4\n  FROM t"),
        ],
    )
}

#[test]
fn top_statements_uses_the_difference_since_the_previous_report() {
    let previous = week_ago();
    let top = top_statements(Some(&previous), &now(), 3);

    assert!(top.since_previous);
    assert_eq!(top.since, at(24, 11));
    assert_eq!(
        top.top,
        vec![
            TopStatement {
                queryid: 4,
                calls: 2,
                total_ms: 60_000.0,
                query: "SELECT 4 FROM t".to_string(),
            },
            TopStatement {
                queryid: 1,
                calls: 20,
                total_ms: 40_000.0,
                query: "WITH chip AS (…)".to_string(),
            },
        ]
    );
}

#[test]
fn top_statements_falls_back_to_totals_after_a_reset_or_without_a_snapshot() {
    let mut previous = week_ago();
    previous.stats_reset = at(1, 0);

    for previous in [Some(&previous), None] {
        let top = top_statements(previous, &now(), 2);
        assert!(!top.since_previous);
        assert_eq!(top.since, at(19, 22));
        assert_eq!(
            top.top.iter().map(|item| item.calls).collect::<Vec<_>>(),
            vec![120, 2]
        );
    }
}

#[test]
fn statement_line_lists_the_top_statements_with_mean_and_total() {
    let previous = week_ago();
    assert_eq!(
        statement_line(Some(&previous), &Ok(Some(now()))),
        "資料庫端耗時前 2（上次週報 10-24 11:00起）：1. SELECT 4 FROM t（#4）2 次、平均 30000 ms、共 60 秒；\
         2. WITH chip AS (…)（#1）20 次、平均 2000 ms、共 40 秒"
    );
}

#[test]
fn statement_line_truncates_long_statements() {
    let long = format!("SELECT {}", "x, ".repeat(40));
    let line = statement_line(
        None,
        &Ok(Some(stats(at(19, 22), vec![stat(9, 1, 1.0, &long)]))),
    );
    let head: String = long.chars().take(48).collect();

    assert!(line.contains(&format!("{head}…（#9）1 次")), "{line}");
}

#[test]
fn statement_line_reports_missing_extension_failures_and_quiet_weeks() {
    let previous = SavedStatements::new(&now(), at(29, 11));

    assert_eq!(
        statement_line(None, &Ok(None)),
        "資料庫端耗時：沒有安裝 pg_stat_statements"
    );
    assert_eq!(
        statement_line(None, &Err(())),
        "資料庫端耗時：讀取 pg_stat_statements 失敗（見日誌）"
    );
    assert_eq!(
        statement_line(Some(&previous), &Ok(Some(now()))),
        "資料庫端耗時：上次週報 10-29 11:00起沒有新的語句"
    );
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn saved_statements_round_trip_through_redis() {
    dotenvy::dotenv().ok();
    if CLIENT.ping().await.is_err() {
        println!("跳過 saved_statements_round_trip_through_redis：無 Redis 連接");
        return;
    }
    // 不碰正式的快照鍵，免得本機連到正式 Redis 時蓋掉下次週報的起點。
    let key = "test:data_health:pg_stat_statements";
    let current = now();
    save(key, &current).await;
    let loaded = load(key).await.expect("讀回快照");
    CLIENT.delete(key).await.expect("清除測試鍵");

    assert_eq!(loaded.stats_reset, current.stats_reset);
    assert_eq!(
        loaded.totals,
        SavedStatements::new(&current, at(1, 0)).totals
    );
}
