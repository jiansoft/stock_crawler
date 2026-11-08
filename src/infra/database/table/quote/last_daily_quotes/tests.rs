//! 最後交易日報價表的測試。

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_calculate() {
    dotenvy::dotenv().ok();
    tracing::info!("開始 fetch");
    let _ = LastDailyQuotes::new();
    match LastDailyQuotes::fetch().await {
        Ok(stocks) => tracing::info!("{:#?}", stocks),
        Err(why) => {
            tracing::error!("Failed to fetch because {:?}", why);
        }
    }

    tracing::info!("結束 fetch");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_rebuild() {
    dotenvy::dotenv().ok();
    tracing::info!("開始 rebuild");
    match LastDailyQuotes::rebuild().await {
        Ok(r) => tracing::info!("{:#?}", r),
        Err(why) => {
            tracing::error!("Failed to rebuild because {:?}", why);
        }
    }

    tracing::info!("結束 rebuild");
}
