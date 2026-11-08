//! 排程入口與手動補記入口（臨時資料庫）：沒有要通知的策略時略過、補記時查無策略回錯誤。

use super::*;
use crate::infra::database;

/// 測試用具名策略（`alert = false`）。
const PRESET: &str = "測試策略-79979";

async fn cleanup() {
    sqlx::query("DELETE FROM strategy_preset WHERE name = $1")
        .bind(PRESET)
        .execute(database::get_connection())
        .await
        .expect("清除測試策略");
}

/// 具名策略都沒開通知、也沒設環境變數時直接略過，不碰回測閘道。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn execute_skips_when_no_strategy_alerts() {
    dotenvy::dotenv().ok();
    let pool = database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 execute_skips_when_no_strategy_alerts：無資料庫連接");
        return;
    }
    if std::env::var(QUERY_ENV).is_ok_and(|query| !query.trim().is_empty()) {
        println!("跳過 execute_skips_when_no_strategy_alerts：已設定 {QUERY_ENV}");
        return;
    }
    let alerting: i64 = sqlx::query_scalar("SELECT count(*) FROM strategy_preset WHERE alert")
        .fetch_one(pool)
        .await
        .expect("查詢要通知的策略");
    if alerting > 0 {
        println!("跳過 execute_skips_when_no_strategy_alerts：資料庫已有要通知的策略");
        return;
    }
    cleanup().await;
    sqlx::query(
        "INSERT INTO strategy_preset (name, query, alert) VALUES ($1, 'from=1999-01-01', false)",
    )
    .bind(PRESET)
    .execute(pool)
    .await
    .expect("插入測試策略");

    let result = execute().await;
    cleanup().await;

    result.expect("沒有要通知的策略時略過");
}

/// 手動補記：策略不存在時回錯誤並指出名稱。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn record_current_rebalance_fails_for_unknown_preset() {
    dotenvy::dotenv().ok();
    if sqlx::query("SELECT 1")
        .execute(database::get_connection())
        .await
        .is_err()
    {
        println!("跳過 record_current_rebalance_fails_for_unknown_preset：無資料庫連接");
        return;
    }
    cleanup().await;

    let error = record_current_rebalance(PRESET)
        .await
        .expect_err("查無策略應回錯誤");

    assert!(error.to_string().contains(PRESET), "{error:#}");
}
