//! 殖利率排行倉儲的測試。

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_pg_yield_rank_repository_contract() {
    // 這是一個單元合約測試佔位符，如果沒有資料庫連接則跳過
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 PgYieldRankRepository DB 整合測試：無資料庫連接");
        return;
    }

    let repo = PgYieldRankRepository::new();
    let test_date = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();

    // 呼叫重建函式驗證基本 SQL 是否正常執行
    let result = repo.rebuild_by_date(test_date).await;
    if let Err(ref e) = result {
        println!("Error during rebuild_by_date: {:?}", e);
    }
    assert!(result.is_ok());
}
