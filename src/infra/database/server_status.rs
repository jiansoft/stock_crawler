//! 資料庫伺服器狀態：啟動時間與 `pg_stat_statements` 統計重置時間。
//!
//! 停機監控不能用共用連線池：資料庫掛掉時取連線要等 acquire timeout，還會占住排程要用的連線。
//! 這裡每次開一條獨立連線、查完就關，整段受呼叫端給的逾時限制。

use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use sqlx::{Connection, PgConnection, postgres::PgConnectOptions};

/// 伺服器啟動時間。
const STARTED_AT_SQL: &str = "SELECT pg_postmaster_start_time()";
/// `pg_stat_statements` 統計上次重置的時間；沒有安裝擴充時查詢失敗。
const STATS_RESET_SQL: &str = "SELECT stats_reset FROM pg_stat_statements_info";

/// 一次探測讀到的伺服器狀態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerStatus {
    /// `pg_postmaster_start_time()`。
    pub started_at: DateTime<Local>,
    /// `pg_stat_statements_info.stats_reset`；沒有安裝擴充或沒有權限時為 `None`。
    pub stats_reset: Option<DateTime<Local>>,
}

/// 以正式設定的連線參數探測伺服器狀態。
///
/// # Errors
///
/// 連線失敗、查詢失敗或超過 `timeout` 時回傳錯誤。
pub async fn probe(timeout: Duration) -> Result<ServerStatus> {
    probe_with(&super::connect_options(), timeout).await
}

/// 以指定的連線參數探測伺服器狀態（測試可指向不存在的主機）。
///
/// # Errors
///
/// 連線失敗、查詢失敗或超過 `timeout` 時回傳錯誤。
pub(crate) async fn probe_with(
    options: &PgConnectOptions,
    timeout: Duration,
) -> Result<ServerStatus> {
    tokio::time::timeout(timeout, read_status(options))
        .await
        .with_context(|| format!("超過 {} 秒沒有回應", timeout.as_secs()))?
}

async fn read_status(options: &PgConnectOptions) -> Result<ServerStatus> {
    let mut conn = PgConnection::connect_with(options)
        .await
        .context("連線失敗")?;
    let started_at: DateTime<Local> = sqlx::query_scalar(STARTED_AT_SQL)
        .fetch_one(&mut conn)
        .await
        .context("讀取伺服器啟動時間失敗")?;
    let stats_reset: Option<DateTime<Local>> = sqlx::query_scalar(STATS_RESET_SQL)
        .fetch_optional(&mut conn)
        .await
        .ok()
        .flatten();
    if let Err(why) = conn.close().await {
        tracing::debug!("資料庫健康檢查：關閉連線失敗：{why}");
    }
    Ok(ServerStatus {
        started_at,
        stats_reset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 連不上的主機在逾時內回傳錯誤，不會卡住。
    #[tokio::test]
    async fn probe_with_unreachable_host_fails() {
        let options = PgConnectOptions::new()
            .host("127.0.0.1")
            .port(1)
            .username("nobody")
            .database("nothing");

        let result = probe_with(&options, Duration::from_secs(5)).await;

        assert!(result.is_err(), "port 1 沒有 PostgreSQL，應該回錯誤");
    }

    /// 逾時以秒數說明。
    #[tokio::test]
    async fn probe_with_times_out() {
        // 192.0.2.0/24 是文件保留網段（RFC 5737），連線會一直等到逾時或被拒。
        let options = PgConnectOptions::new()
            .host("192.0.2.1")
            .port(5432)
            .username("nobody")
            .database("nothing");

        let result = probe_with(&options, Duration::from_millis(200)).await;

        let error = format!("{:#}", result.expect_err("保留網段不應連得上"));
        assert!(
            error.contains("沒有回應") || error.contains("連線失敗"),
            "錯誤要說明是逾時或連線失敗：{error}"
        );
    }

    /// 連得上時讀到啟動時間，且早於現在。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn probe_reads_started_at() {
        dotenvy::dotenv().ok();
        let Ok(status) = probe(Duration::from_secs(5)).await else {
            println!("跳過 probe_reads_started_at：無資料庫連接");
            return;
        };

        assert!(
            status.started_at <= Local::now(),
            "啟動時間不應晚於現在：{status:?}"
        );
    }
}
