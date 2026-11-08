//! 估價計算的測試。

use crate::infra::cache::SHARE;

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_calculate_estimated_price() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 calculate_estimated_price");
    let current_date = NaiveDate::parse_from_str("2026-03-31", "%Y-%m-%d").unwrap();
    match calculate_estimated_price(current_date).await {
        Ok(_) => {}
        Err(why) => {
            tracing::debug!("Failed to calculate_estimated_price because {:?}", why);
        }
    }
    tracing::debug!("結束 calculate_estimated_price");
}
