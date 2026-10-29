//! 資料庫端各語句的累計耗時（`pg_stat_statements`）。
//!
//! 檔案日誌的慢查詢只記到超過門檻的單筆（一般池 1 秒、批次池 5 秒），每次 0.8 秒、一天跑幾百次的
//! 查詢永遠不會出現；這裡讀資料庫自己的累計值，週報用前後兩次的差額找出本週最耗時的語句。

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use sqlx::PgConnection;

use crate::{
    domain::health::{StatementStat, StatementStats},
    infra::database,
};

/// 擴充是否已安裝在目前的資料庫。
const EXTENSION_INSTALLED_SQL: &str =
    "SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'pg_stat_statements')";

/// 統計上次重置的時間（1.9 版起才有 `pg_stat_statements_info`）。
const STATS_RESET_SQL: &str = "SELECT stats_reset FROM pg_stat_statements_info";

/// 目前資料庫的最上層語句，不同使用者的同一語句合併。
const STATEMENTS_SQL: &str = r#"
SELECT s.queryid,
       SUM(s.calls)::bigint AS calls,
       SUM(s.total_exec_time)::float8 AS total_ms,
       MIN(s.query) AS query
FROM pg_stat_statements AS s
WHERE s.dbid = (SELECT oid FROM pg_database WHERE datname = current_database())
  AND s.toplevel
  AND s.queryid IS NOT NULL
GROUP BY s.queryid
"#;

/// 沒有安裝擴充時回 `None`；裝了卻讀不到（例如沒有 `shared_preload_libraries`）時回錯誤。
pub(super) async fn fetch_statement_stats() -> Result<Option<StatementStats>> {
    let mut conn = database::get_connection()
        .acquire()
        .await
        .context("Failed to acquire connection for pg_stat_statements")?;
    let installed: bool = sqlx::query_scalar(EXTENSION_INSTALLED_SQL)
        .fetch_one(&mut *conn)
        .await
        .context("Failed to check pg_stat_statements extension")?;
    if !installed {
        return Ok(None);
    }
    read_statements(&mut conn).await.map(Some)
}

/// 讀統計起算時間與各語句的累計值。
pub(super) async fn read_statements(conn: &mut PgConnection) -> Result<StatementStats> {
    let stats_reset: DateTime<Local> = sqlx::query_scalar(STATS_RESET_SQL)
        .fetch_one(&mut *conn)
        .await
        .context("Failed to read pg_stat_statements_info")?;
    let rows: Vec<(i64, i64, f64, Option<String>)> = sqlx::query_as(STATEMENTS_SQL)
        .fetch_all(&mut *conn)
        .await
        .context("Failed to read pg_stat_statements")?;
    let rows = rows
        .into_iter()
        .map(|(queryid, calls, total_ms, query)| StatementStat {
            queryid,
            calls,
            total_ms,
            query: query.unwrap_or_default(),
        })
        .collect();
    Ok(StatementStats { stats_reset, rows })
}
