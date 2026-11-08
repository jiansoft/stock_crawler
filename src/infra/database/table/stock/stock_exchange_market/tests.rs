//! 交易所市場別對照表的測試。

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 StockExchangeMarket::fetch");
    println!("開始 StockExchangeMarket::fetch");

    match StockExchangeMarket::fetch().await {
        Ok(markets) => {
            println!("markets:{:#?}", markets);
            tracing::debug!("markets:{:#?}", markets);
        }
        Err(why) => {
            println!("error:{:#?}", why);
            tracing::debug!("{:?}", why);
        }
    }
    tracing::debug!("結束 StockExchangeMarket::fetch");
}
