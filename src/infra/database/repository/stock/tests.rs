use super::*;
use rust_decimal_macros::dec;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_pg_stock_repository_contract() {
    // 這是一個單元合約測試佔位符，如果沒有資料庫連接則跳過
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 PgStockRepository DB 整合測試：無資料庫連接");
        return;
    }

    let repo = PgStockRepository::new();
    let test_symbol = "__TEST_REPO_STOCK__";

    // 清理
    sqlx::query("DELETE FROM stocks WHERE stock_symbol = $1")
        .bind(test_symbol)
        .execute(database::get_connection())
        .await
        .ok();

    let new_stock = Stock::register(test_symbol.to_string(), "測試倉儲".to_string(), 2, 24);

    repo.save(&new_stock).await.unwrap();

    let fetched = repo.find_by_symbol(test_symbol).await.unwrap().unwrap();
    assert_eq!(fetched.symbol().0, test_symbol);
    assert_eq!(fetched.name(), "測試倉儲");
    assert_eq!(fetched.market_id(), 2);
    assert_eq!(fetched.industry_id(), 24);

    // 更新測試
    let mut updated = fetched;
    updated.change_identity("測試倉儲更新".to_string(), 4, 25);
    updated.update_net_asset_value(dec!(100.5));
    repo.save(&updated).await.unwrap();

    let fetched_updated = repo.find_by_symbol(test_symbol).await.unwrap().unwrap();
    assert_eq!(fetched_updated.name(), "測試倉儲更新");
    assert_eq!(fetched_updated.market_id(), 4);
    assert_eq!(fetched_updated.industry_id(), 25);
    assert_eq!(fetched_updated.net_asset_value_per_share(), dec!(100.5));

    // 清理
    sqlx::query("DELETE FROM stocks WHERE stock_symbol = $1")
        .bind(test_symbol)
        .execute(database::get_connection())
        .await
        .unwrap();
}
