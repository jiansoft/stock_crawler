//! annual_profit的測試；連外的測試標 #[ignore]。

use super::*;

#[tokio::test]
#[ignore = "live test：連線真實外部網站，需要時手動執行"]
async fn test_visit() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 visit");

    match visit("2330").await {
        Ok(e) => {
            dbg!(&e);
            tracing::debug!("moneydj : {:#?}", e);
        }
        Err(why) => {
            tracing::debug!("Failed to visit because {:?}", why);
        }
    }

    tracing::debug!("結束 visit");
}
