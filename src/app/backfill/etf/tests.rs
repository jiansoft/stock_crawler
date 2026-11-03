//! 台股 ETF 資訊同步的測試。

use super::*;
use crate::core::declare::StockExchangeMarket;

const LISTED: &str = "79979EA";
const OTC: &str = "79979EB";
const PUBLIC: &str = "79979EP";

fn etf(symbol: &str, name: &str, market: StockExchangeMarket) -> EtfInfo {
    EtfInfo {
        stock_symbol: symbol.to_owned(),
        name: name.to_owned(),
        listing_date: String::new(),
        industry: "ETF".to_owned(),
        market,
        industry_id: 9001,
    }
}

/// 上市、上櫃各自寫入；市場別對照表沒有的（公開發行）略過；資料相同不重寫、改名再寫入。
/// 假代號結束時刪除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn update_stocks_registers_new_and_renamed_etfs() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 update_stocks_registers_new_and_renamed_etfs：無資料庫連接");
        return;
    }
    SHARE.load().await;
    let cleanup = || async {
        sqlx::query("DELETE FROM stocks WHERE stock_symbol = ANY($1)")
            .bind([LISTED, OTC, PUBLIC])
            .execute(pool)
            .await
            .expect("清除測試股票");
    };
    cleanup().await;
    let names = || async {
        sqlx::query_as::<_, (String, String, i32)>(
            r#"SELECT stock_symbol, "Name", stock_exchange_market_id FROM stocks
               WHERE stock_symbol = ANY($1) ORDER BY stock_symbol"#,
        )
        .bind([LISTED, OTC, PUBLIC])
        .fetch_all(pool)
        .await
        .expect("查詢測試股票")
    };

    update_stocks(vec![
        etf(LISTED, "測試上市ETF", StockExchangeMarket::Listed),
        etf(PUBLIC, "測試公開發行", StockExchangeMarket::Public),
        etf(OTC, "測試上櫃ETF", StockExchangeMarket::OverTheCounter),
    ])
    .await
    .expect("寫入 ETF");
    let first = names().await;

    // 資料相同的略過、改名的再寫入。
    update_stocks(vec![
        etf(LISTED, "測試改名ETF", StockExchangeMarket::Listed),
        etf(OTC, "測試上櫃ETF", StockExchangeMarket::OverTheCounter),
    ])
    .await
    .expect("更新 ETF");
    let second = names().await;
    cleanup().await;

    assert_eq!(
        first,
        [
            (LISTED.to_owned(), "測試上市ETF".to_owned(), 2),
            (OTC.to_owned(), "測試上櫃ETF".to_owned(), 4),
        ]
    );
    assert_eq!(
        second,
        [
            (LISTED.to_owned(), "測試改名ETF".to_owned(), 2),
            (OTC.to_owned(), "測試上櫃ETF".to_owned(), 4),
        ]
    );
}

/// 手動執行：抓官方 ETF 清單並更新正式資料（需要網路）。
#[tokio::test]
#[ignore]
async fn test_execute_etf() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 execute_etf");

    match execute().await {
        Ok(_) => {
            tracing::debug!("完成 execute_etf");
        }
        Err(why) => {
            tracing::debug!("執行失敗: {:?}", why);
        }
    }
}
