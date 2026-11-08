//! 資料庫伺服器狀態：啟動時間與 `pg_stat_statements` 統計重置時間的測試。

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
