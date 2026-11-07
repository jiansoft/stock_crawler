//! price的測試；連外的測試標 #[ignore]。

use crate::infra::crawler::log_stock_price_test;

use super::*;

#[tokio::test]
#[ignore = "live test：連線真實外部網站，需要時手動執行"]
async fn test_get_stock_price() {
    dotenvy::dotenv().ok();
    log_stock_price_test::<NStock>("2330").await;
}

#[tokio::test]
#[ignore = "live test：連線真實外部網站，需要時手動執行"]
async fn test_get_stock_quotes() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 nstock::get_stock_quotes");

    match NStock::get_stock_quotes("2330").await {
        Ok(e) => {
            dbg!(&e);
            tracing::debug!("nstock::get_stock_quotes : {:#?}", e);
        }
        Err(why) => {
            tracing::debug!("Failed to nstock::get_stock_quotes because {:?}", why);
        }
    }

    tracing::debug!("結束 nstock::get_stock_quotes");
}
