//! 系統設定倉儲的測試。

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_pg_config_repository_contract() {
    // 這是一個單元合約測試佔位符，如果沒有資料庫連接則跳過
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 PgConfigRepository DB 整合測試：無資料庫連接");
        return;
    }

    let repo = PgConfigRepository::new();
    let test_key = "__TEST_CONFIG_KEY__";

    // 1. 清理潛在的殘留測試數據
    sqlx::query("DELETE FROM config WHERE key = $1")
        .bind(test_key)
        .execute(database::get_connection())
        .await
        .ok();

    // 2. 寫入並保存測試設定
    let config = SystemConfig::new(test_key.to_string(), "測試值".to_string());
    repo.save(&config).await.unwrap();

    // 3. 讀取並驗證
    let fetched = repo.find_by_key(test_key).await.unwrap().unwrap();
    assert_eq!(fetched.key, test_key);
    assert_eq!(fetched.val, "測試值");

    // 4. 清理測試數據
    sqlx::query("DELETE FROM config WHERE key = $1")
        .bind(test_key)
        .execute(database::get_connection())
        .await
        .unwrap();
}
