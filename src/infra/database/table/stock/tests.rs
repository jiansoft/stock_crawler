use rust_decimal_macros::dec;

use crate::domain::registry::repository::StockRepository;

use super::*;

#[test]
fn is_preference_shares_detects_letters_only() {
    assert!(!is_preference_shares("2330"));
    assert!(!is_preference_shares("0050"));
    assert!(is_preference_shares("2881A"));
    assert!(is_preference_shares("abc"));
    assert!(is_preference_shares("123b"));
}

#[test]
fn stock_methods_key_tdr_and_stock_info_request() {
    let mut stock = StockDbRow::new();
    stock.stock_symbol = "9105".to_string();
    stock.name = "泰金寶-DR".to_string();
    stock.stock_exchange_market_id = 2;
    stock.stock_industry_id = 20;
    stock.net_asset_value_per_share = dec!(12.34);
    stock.suspend_listing = true;

    assert_eq!(stock.key(), "9105");
    assert_eq!(stock.key_with_prefix(), "Stock:9105");
}

#[test]
fn stock_clone_preserves_all_fields() {
    let mut stock = StockDbRow::new();
    stock.stock_symbol = "2330".to_string();
    stock.name = "台積電".to_string();
    stock.suspend_listing = true;
    stock.net_asset_value_per_share = dec!(95.12);
    stock.weight = dec!(31.5);
    stock.return_on_equity = dec!(25.7);
    stock.stock_exchange_market_id = 2;
    stock.stock_industry_id = 24;
    stock.issued_share = 25_000;
    stock.qfii_shares_held = 12_000;
    stock.qfii_share_holding_percentage = dec!(48.5);

    let cloned = stock.clone();

    assert_eq!(cloned.stock_symbol, stock.stock_symbol);
    assert_eq!(cloned.name, stock.name);
    assert_eq!(cloned.suspend_listing, stock.suspend_listing);
    assert_eq!(
        cloned.net_asset_value_per_share,
        stock.net_asset_value_per_share
    );
    assert_eq!(cloned.weight, stock.weight);
    assert_eq!(cloned.return_on_equity, stock.return_on_equity);
    assert_eq!(cloned.create_time, stock.create_time);
    assert_eq!(
        cloned.stock_exchange_market_id,
        stock.stock_exchange_market_id
    );
    assert_eq!(cloned.stock_industry_id, stock.stock_industry_id);
    assert_eq!(cloned.issued_share, stock.issued_share);
    assert_eq!(cloned.qfii_shares_held, stock.qfii_shares_held);
    assert_eq!(
        cloned.qfii_share_holding_percentage,
        stock.qfii_share_holding_percentage
    );
}

// 此測試驗證防禦性 upsert 邏輯（當新傳入的市場或產業編號為 0 時，保留資料庫中原先正確的非零值）。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_upsert_industry() {
    dotenvy::dotenv().ok();

    // 若目前測試環境無法連接資料庫，則自動跳過，避免在無資料庫之開發環境下執行單元測試失敗
    if crate::infra::database::ping().await.is_err() {
        println!("跳過 test_upsert_industry：資料庫未啟動或無法連線");
        return;
    }

    let test_symbol = "__TEST_UPSERT_IND_9999__";
    let cleanup_sql = "DELETE FROM stocks WHERE stock_symbol = $1;";

    // 1. 確保乾淨的測試起點，先清除可能存在的舊測試資料
    sqlx::query(cleanup_sql)
        .bind(test_symbol)
        .execute(crate::infra::database::get_connection())
        .await
        .ok();

    let repo = crate::infra::database::repository::stock::PgStockRepository::new();

    // 2. 寫入初始的測試股票資料，並給予非零之市場與產業編號
    let seed = crate::domain::registry::entity::Stock::reconstitute(
        test_symbol.to_string(),
        "測試防禦更新股".to_string(),
        false,
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
        Local::now(),
        4,
        15,
        0,
        0,
        Decimal::ZERO,
    );
    repo.save(&seed).await.expect("Failed to insert seed stock");

    // 3. 測試防禦邏輯：呼叫 upsert 時傳入 0 的市場與產業 ID
    let update_zero = crate::domain::registry::entity::Stock::reconstitute(
        test_symbol.to_string(),
        "測試防禦更新股_已更新".to_string(),
        false,
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
        Local::now(),
        0,
        0,
        0,
        0,
        Decimal::ZERO,
    );
    repo.save(&update_zero)
        .await
        .expect("Failed to upsert zero fields");

    // 4. 驗證防禦更新：確認原先的非零值 (4 與 15) 被妥善保留，並未被 0 覆蓋
    let row1 = sqlx::query("SELECT \"Name\", stock_exchange_market_id, stock_industry_id FROM stocks WHERE stock_symbol = $1")
        .bind(test_symbol)
        .fetch_one(crate::infra::database::get_connection())
        .await
        .expect("Failed to fetch updated stock");
    assert_eq!(row1.get::<String, _>("Name"), "測試防禦更新股_已更新");
    assert_eq!(row1.get::<i32, _>("stock_exchange_market_id"), 4);
    assert_eq!(row1.get::<i32, _>("stock_industry_id"), 15);

    // 5. 測試正常覆寫：呼叫 upsert 時傳入新的非零之市場與產業 ID
    let update_new = crate::domain::registry::entity::Stock::reconstitute(
        test_symbol.to_string(),
        "測試防禦更新股_二次更新".to_string(),
        false,
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
        Local::now(),
        2,
        8,
        0,
        0,
        Decimal::ZERO,
    );
    repo.save(&update_new)
        .await
        .expect("Failed to upsert new fields");

    // 6. 驗證覆寫更新：確認市場與產業 ID 已經被正確更新為新的值 (2 與 8)
    let row2 = sqlx::query("SELECT \"Name\", stock_exchange_market_id, stock_industry_id FROM stocks WHERE stock_symbol = $1")
        .bind(test_symbol)
        .fetch_one(crate::infra::database::get_connection())
        .await
        .expect("Failed to fetch second updated stock");
    assert_eq!(row2.get::<String, _>("Name"), "測試防禦更新股_二次更新");
    assert_eq!(row2.get::<i32, _>("stock_exchange_market_id"), 2);
    assert_eq!(row2.get::<i32, _>("stock_industry_id"), 8);

    // 7. 清理測試資料，還原資料庫狀態
    sqlx::query(cleanup_sql)
        .bind(test_symbol)
        .execute(crate::infra::database::get_connection())
        .await
        .expect("Failed to cleanup test stock");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_update_last_eps() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 update_last_eps");
    match StockDbRow::update_eps_and_roe().await {
        Ok(_) => {}
        Err(why) => {
            tracing::debug!("Failed to update_last_eps because: {:?}", why);
        }
    }

    tracing::debug!("結束 update_last_eps");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 StockDbRow::fetch");
    match StockDbRow::fetch().await {
        Ok(stocks) => {
            tracing::debug!("stocks:{:#?}", stocks);
        }
        Err(why) => {
            tracing::debug!("{:?}", why);
        }
    }
    tracing::debug!("結束 StockDbRow::fetch");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_net_asset_value_per_share_is_zero() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_net_asset_value_per_share_is_zero");
    match fetch_net_asset_value_per_share_is_zero().await {
        Ok(stocks) => {
            for e in stocks {
                tracing::debug!("{} {:?} ", is_preference_shares(&e.stock_symbol), e);
            }
        }
        Err(why) => {
            tracing::debug!(
                "Failed to fetch_net_asset_value_per_share_is_zero because: {:?}",
                why
            );
        }
    }

    tracing::debug!("結束 fetch_net_asset_value_per_share_is_zero");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_stocks_without_financial_statement() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_stocks_without_financial_statement");
    match fetch_stocks_without_financial_statement(2022, "Q4").await {
        Ok(stocks) => {
            for e in stocks {
                tracing::debug!("{} {:?} ", is_preference_shares(&e.stock_symbol), e);
            }
        }
        Err(why) => {
            tracing::debug!(
                "Failed to fetch_stocks_without_financial_statement because: {:?}",
                why
            );
        }
    }

    tracing::debug!("結束 fetch_stocks_without_financial_statement");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_create_index() {
    dotenvy::dotenv().ok();
    create_search_index("2330", "台積電").await;
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_rebuild_search_indices() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 rebuild_search_indices");

    match StockDbRow::rebuild_search_indices().await {
        Ok(_) => {
            tracing::debug!("完成 rebuild_search_indices");
        }
        Err(why) => {
            tracing::debug!("Failed to rebuild_search_indices because: {:?}", why);
        }
    }

    tracing::debug!("結束 rebuild_search_indices");
}
