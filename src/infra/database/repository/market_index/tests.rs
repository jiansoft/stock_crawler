//! 大盤指數倉儲的測試。

use super::*;
use rust_decimal_macros::dec;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_pg_market_index_repository_contract() {
    // 這是一個單元合約測試佔位符，如果沒有資料庫連接則跳過
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 PgMarketIndexRepository DB 整合測試：無資料庫連接");
        return;
    }

    let repo = PgMarketIndexRepository::new();
    let test_category = "__TEST_INDEX__";
    let test_date = NaiveDate::from_ymd_opt(2099, 12, 31).unwrap();

    // 1. 清理潛在殘留數據
    sqlx::query("DELETE FROM \"index\" WHERE category = $1 AND date = $2")
        .bind(test_category)
        .bind(test_date)
        .execute(database::get_connection())
        .await
        .ok();

    // 2. 建立新實體並保存
    let market_index = MarketIndex::new(
        test_category.to_string(),
        test_date,
        dec!(16000.5),
        dec!(150.25),
        dec!(5000000000),
        dec!(200000),
        dec!(300000000),
    );

    repo.save(&market_index).await.unwrap();

    // 3. 獲取最新資料並驗證
    let latest = repo.fetch_latest(10).await.unwrap();
    let found = latest
        .iter()
        .find(|idx| idx.category == test_category && idx.date == test_date);
    assert!(found.is_some());
    let found_index = found.unwrap();
    assert_eq!(found_index.index, dec!(16000.5));
    assert_eq!(found_index.change, dec!(150.25));
    assert_eq!(found_index.trade_value, dec!(5000000000));
    assert_eq!(found_index.transaction, dec!(200000));
    assert_eq!(found_index.trading_volume, dec!(300000000));

    // 收盤查詢：含當天、由新到舊；指定日期之前沒有資料回空。
    let closes = repo
        .fetch_index_closes(test_category, Some(test_date), 5)
        .await
        .unwrap();
    assert_eq!(closes, vec![(test_date, 16000.5)]);
    let before = repo
        .fetch_index_closes(test_category, test_date.pred_opt(), 5)
        .await
        .unwrap();
    assert!(before.is_empty());
    let latest_closes = repo
        .fetch_index_closes(test_category, None, 1)
        .await
        .unwrap();
    assert_eq!(latest_closes, vec![(test_date, 16000.5)]);

    // 4. 清理
    sqlx::query("DELETE FROM \"index\" WHERE category = $1 AND date = $2")
        .bind(test_category)
        .bind(test_date)
        .execute(database::get_connection())
        .await
        .unwrap();
}
