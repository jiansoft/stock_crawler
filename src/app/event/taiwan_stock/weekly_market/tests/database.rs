//! 每週市場摘要排程入口連資料庫的整合測試。

use super::*;

/// 排程入口連資料庫走完：讀持股、各段查詢、組訊息；資料不足的段落只降級、不失敗。
/// 只讀資料庫；測試程式沒有註冊通知 sink，組出訊息也只記警告、不會真的發送。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn execute_runs_against_database() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 execute_runs_against_database：無資料庫連接");
        return;
    }

    execute().await.expect("各段失敗只降級，排程入口照常結束");
}
