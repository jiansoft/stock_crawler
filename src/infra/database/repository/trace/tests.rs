//! 股價追蹤倉儲的測試。

use super::*;
use rust_decimal_macros::dec;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_pg_trace_repository_contract() {
    // 這是一個單元合約測試佔位符，如果沒有資料庫連接則跳過
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 PgTraceRepository DB 整合測試：無資料庫連接");
        return;
    }

    let repo = PgTraceRepository::new();
    let test_symbol = "__TEST_TRACE__";

    // 1. 清理潛在的殘留測試數據
    sqlx::query("DELETE FROM \"trace\" WHERE stock_symbol = $1")
        .bind(test_symbol)
        .execute(database::get_connection())
        .await
        .ok();

    // 2. 寫入一筆測試資料
    sqlx::query("INSERT INTO \"trace\" (stock_symbol, floor, ceiling) VALUES ($1, $2, $3)")
        .bind(test_symbol)
        .bind(dec!(100.5))
        .bind(dec!(200.5))
        .execute(database::get_connection())
        .await
        .unwrap();

    // 3. 呼叫倉儲的 fetch_all 驗證是否能載入該測試資料
    let traces = repo.fetch_all().await.unwrap();
    let found = traces.iter().find(|t| t.stock_symbol == test_symbol);
    assert!(found.is_some());
    let found_trace = found.unwrap();
    assert_eq!(found_trace.floor, dec!(100.5));
    assert_eq!(found_trace.ceiling, dec!(200.5));

    // 4. 清理測試數據
    sqlx::query("DELETE FROM \"trace\" WHERE stock_symbol = $1")
        .bind(test_symbol)
        .execute(database::get_connection())
        .await
        .unwrap();
}
