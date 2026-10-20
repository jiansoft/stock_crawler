//! 連線 HiStock 正式站的手動測試（`#[ignore]`）。

use super::*;

#[tokio::test]
#[ignore]
async fn test_start_caching_task_integration() {
    let _guard = TEST_STATE_LOCK.lock().await;
    stop_caching_task().await;
    start_caching_task();

    println!("Waiting for background fetch...");
    tokio::time::sleep(Duration::from_secs(6)).await;

    {
        let cache = SHARE.stock_snapshots.read().unwrap();
        assert!(!cache.is_empty());
        println!("Background cache populated with {} stocks", cache.len());
    }

    let price = HiStock::get_stock_price("2330").await.unwrap();
    assert!(price > Decimal::ZERO);

    stop_caching_task().await;
}

#[tokio::test]
#[ignore]
async fn test_get_stock_price() {
    let _guard = TEST_STATE_LOCK.lock().await;
    dotenvy::dotenv().ok();
    tracing::debug!("開始 HiStock::get_stock_price");

    match HiStock::get_stock_price("2330").await {
        Ok(e) => {
            dbg!(&e);
            tracing::debug!("HiStock::get_stock_price : {:#?}", e);
        }
        Err(why) => {
            tracing::debug!("Failed to visit because {:?}", why);
        }
    }
}

#[tokio::test]
#[ignore]
async fn test_get_stock_quotes() {
    let _guard = TEST_STATE_LOCK.lock().await;
    dotenvy::dotenv().ok();
    tracing::debug!("開始 HiStock::get_stock_quotes");

    match HiStock::get_stock_quotes("2330").await {
        Ok(e) => {
            dbg!(&e);
            tracing::debug!("HiStock::get_stock_quotes : {:#?}", e);
        }
        Err(why) => {
            tracing::debug!("Failed to HiStock::get_stock_quotes because {:?}", why);
        }
    }
}

#[tokio::test]
#[ignore]
async fn test_get_stock_price_with_cache_verification() {
    let _guard = TEST_STATE_LOCK.lock().await;
    // 1. 清空快取
    stop_caching_task().await;

    // 2. 抓取全量並填充快取
    let all_stocks = fetch_all_from_rank().await.unwrap().snapshots;
    SHARE.set_stock_snapshots(all_stocks);

    // 3. 測試透過公用介面取得價格 (此時應從快取秒讀)
    let price = HiStock::get_stock_price("2330").await.unwrap();
    assert!(price > Decimal::ZERO);
    println!("Verified Price from Cache: {}", price);
}

#[tokio::test]
#[ignore]
async fn test_get_stock_quotes_with_cache_verification() {
    let _guard = TEST_STATE_LOCK.lock().await;
    // 1. 清空快取
    stop_caching_task().await;

    // 2. 抓取全量並填充快取
    let all_stocks = fetch_all_from_rank().await.unwrap().snapshots;
    SHARE.set_stock_snapshots(all_stocks);

    // 3. 測試透過公用介面取得報價物件
    let quotes = HiStock::get_stock_quotes("2330").await.unwrap();
    assert_eq!(quotes.stock_symbol, "2330");
    assert!(quotes.price > 0.0);
    println!("Verified Quotes from Cache: {:?}", quotes);
}
