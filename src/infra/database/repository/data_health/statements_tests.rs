//! `pg_stat_statements` 讀取（[`statements`]）的測試。
//!
//! 測試庫沒有預先載入這個擴充，交易內以同名的暫存表代替：暫存 schema 排在搜尋路徑最前面，
//! 查詢會讀到暫存表；rollback 後不留痕跡。

use chrono::DateTime;

use super::statements::read_statements;
use super::*;
use crate::domain::health::StatementStat;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn read_statements_merges_users_and_skips_nested_and_other_databases() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!(
            "跳過 read_statements_merges_users_and_skips_nested_and_other_databases：無資料庫連接"
        );
        return;
    }
    let mut tx = database::get_connection()
        .begin()
        .await
        .expect("開交易應成功");
    for sql in [
        "CREATE TEMP TABLE pg_stat_statements_info (stats_reset timestamptz) ON COMMIT DROP",
        "INSERT INTO pg_stat_statements_info VALUES ('2026-10-19 22:22:34+08')",
        "CREATE TEMP TABLE pg_stat_statements (userid oid, dbid oid, toplevel bool, queryid bigint,
             calls bigint, total_exec_time float8, query text) ON COMMIT DROP",
        // 同一語句兩個使用者合併；巢狀語句、別的資料庫、沒有 queryid 的列不算。
        "INSERT INTO pg_stat_statements
         SELECT u, (SELECT oid FROM pg_database WHERE datname = current_database()), top, q, c, t, txt
         FROM (VALUES (10, true, 1, 3, 30.0, 'SELECT 1'),
                      (11, true, 1, 2, 20.0, 'SELECT 1'),
                      (10, false, 1, 9, 99.0, 'SELECT 1'),
                      (10, true, 2, 1, 5.5, 'SELECT 2'),
                      (10, true, NULL, 1, 1.0, 'SELECT 3')) AS v(u, top, q, c, t, txt)",
        "INSERT INTO pg_stat_statements VALUES (10, 0, true, 3, 7, 70.0, 'SELECT 9')",
    ] {
        sqlx::query(sql)
            .execute(&mut *tx)
            .await
            .expect("建立暫存統計應成功");
    }

    let mut stats = read_statements(&mut tx).await.expect("讀取應成功");
    tx.rollback().await.expect("rollback 應成功");
    stats.rows.sort_by_key(|row| row.queryid);

    assert_eq!(
        stats.stats_reset,
        DateTime::parse_from_rfc3339("2026-10-19T22:22:34+08:00").unwrap()
    );
    assert_eq!(
        stats.rows,
        vec![
            StatementStat {
                queryid: 1,
                calls: 5,
                total_ms: 50.0,
                query: "SELECT 1".to_string(),
            },
            StatementStat {
                queryid: 2,
                calls: 1,
                total_ms: 5.5,
                query: "SELECT 2".to_string(),
            },
        ]
    );
}

/// 測試庫沒有安裝擴充，回 `None` 而不是錯誤。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn fetch_statement_stats_is_none_without_the_extension() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 fetch_statement_stats_is_none_without_the_extension：無資料庫連接");
        return;
    }
    let installed: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'pg_stat_statements')",
    )
    .fetch_one(database::get_connection())
    .await
    .expect("查詢應成功");
    if installed {
        println!("跳過 fetch_statement_stats_is_none_without_the_extension：測試庫裝了擴充");
        return;
    }

    let stats = PgDataHealthRepository::new().fetch_statement_stats().await;
    assert_eq!(stats.expect("查詢應成功"), None);
}
