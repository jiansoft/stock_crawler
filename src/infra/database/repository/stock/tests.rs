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

/// 沒有要寫回的股票時不碰資料庫，直接回報 0 檔。
#[tokio::test]
async fn save_qfii_batch_skips_empty_input() {
    let updated = PgStockRepository::new()
        .save_qfii_batch(&[])
        .await
        .expect("空清單不需要資料庫");
    assert_eq!(updated, 0);
}

/// 逐檔更新權值後全部歸零；假代號結束時刪除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn update_weight_then_zeroed_out_weights() {
    dotenvy::dotenv().ok();
    let pool = database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 update_weight_then_zeroed_out_weights：無資料庫連接");
        return;
    }
    let symbol = "79979WT";
    let cleanup = || async {
        sqlx::query("DELETE FROM stocks WHERE stock_symbol = $1")
            .bind(symbol)
            .execute(pool)
            .await
            .expect("清除測試股票");
    };
    cleanup().await;
    sqlx::query(
        r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol) VALUES ($1, '測試權值', $1)"#,
    )
    .bind(symbol)
    .execute(pool)
    .await
    .expect("插入測試股票");
    let weight = || async {
        sqlx::query_scalar::<_, Decimal>("SELECT weight FROM stocks WHERE stock_symbol = $1")
            .bind(symbol)
            .fetch_one(pool)
            .await
            .expect("讀回權值")
    };

    let repo = PgStockRepository::new();
    let updated = repo.update_weight(symbol, dec!(1.2345)).await;
    let after_update = weight().await;
    let zeroed = repo.zeroed_out_weights().await;
    let after_zero = weight().await;
    cleanup().await;

    updated.expect("更新權值");
    zeroed.expect("權值歸零");
    assert_eq!(after_update, dec!(1.2345));
    assert_eq!(after_zero, Decimal::ZERO);
}

/// 只取未下市的證券並還原成聚合根（市場、產業、外資持股欄位）；下市的不列。假代號結束時刪除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn fetch_all_active_skips_suspended_stocks() {
    dotenvy::dotenv().ok();
    let pool = database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 fetch_all_active_skips_suspended_stocks：無資料庫連接");
        return;
    }
    let (active, suspended) = ("79979SA", "79979SB");
    let cleanup = || async {
        sqlx::query("DELETE FROM stocks WHERE stock_symbol = ANY($1)")
            .bind(vec![active.to_string(), suspended.to_string()])
            .execute(pool)
            .await
            .expect("清除測試股票");
    };
    cleanup().await;
    for (symbol, suspend) in [(active, false), (suspended, true)] {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, "SuspendListing",
                   stock_exchange_market_id, stock_industry_id, issued_share, qfii_shares_held)
               VALUES ($1, '測試主檔', $1, $2, 2, 24, 1000000, 250000)"#,
        )
        .bind(symbol)
        .bind(suspend)
        .execute(pool)
        .await
        .expect("插入測試股票");
    }

    let stocks = PgStockRepository::default().fetch_all_active().await;
    cleanup().await;

    let stocks = stocks.expect("查詢有效證券");
    let found: Vec<&Stock> = stocks
        .iter()
        .filter(|stock| [active, suspended].contains(&stock.symbol().0.as_str()))
        .collect();
    assert_eq!(found.len(), 1, "下市的不列");
    assert_eq!(found[0].symbol().0, active);
    assert_eq!(found[0].name(), "測試主檔");
    assert_eq!((found[0].market_id(), found[0].industry_id()), (2, 24));
}
