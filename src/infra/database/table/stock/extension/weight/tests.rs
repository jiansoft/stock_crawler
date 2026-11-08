//! 股票權值占比的測試。

use core::result::Result::Ok;

use rust_decimal_macros::dec;

use crate::infra::database::table::stock::StockDbRow;

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_update() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 update");
    let sw = SymbolAndWeight::new("2330".to_string(), dec!(28.3278));
    match sw.update().await {
        Ok(_qr) => {
            let sql = r#"
SELECT
    stock_symbol,
    "Name" AS name,
    "SuspendListing" AS suspend_listing,
    "CreateTime" AS create_time,
    net_asset_value_per_share,
    weight,
    stock_exchange_market_id,
    stock_industry_id
FROM stocks
WHERE stock_symbol = $1;
    "#;

            let stock = sqlx::query_as::<_, StockDbRow>(sql)
                .bind(&sw.stock_symbol)
                .fetch_one(database::get_connection())
                .await;
            match stock {
                Ok(s) => {
                    assert_eq!(s.weight, sw.weight);

                    tracing::debug!("stock:{:?}", s);
                    dbg!(s);
                }
                Err(why) => {
                    tracing::debug!("Failed to fetch stock because: {:?}", why);
                }
            }
        }
        Err(why) => {
            tracing::debug!("Failed to update stock weight because: {:?}", why);
        }
    }

    tracing::debug!("結束 update");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_zeroed_out() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 zeroed_out");

    match SymbolAndWeight::zeroed_out().await {
        Ok(_qr) => {}
        Err(why) => {
            tracing::debug!("Failed to stock weight zeroed_out because: {:?}", why);
        }
    }

    tracing::debug!("結束 zeroed_out");
}
