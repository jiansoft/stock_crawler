//! 市值歷史計算的測試。

use crate::infra::cache::SHARE;

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_calculate_money_history() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 calculate_money_history");
    let current_date = NaiveDate::parse_from_str("2026-04-20", "%Y-%m-%d").unwrap();
    match calculate_money_history(current_date).await {
        Ok(_) => {}
        Err(why) => {
            tracing::debug!("Failed to calculate_money_history because {:?}", why);
        }
    }
    tracing::debug!("結束 calculate_money_history");
}
