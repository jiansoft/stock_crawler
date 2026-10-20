//! 依市場把代號分派到證交所或櫃買中心的抓取端，以及讀快取的市場別。

use super::*;

/// 測試用的市場別：00679B 是上櫃，其他當上市。
fn stub_market_of(stock_symbol: &str) -> Option<i32> {
    (stock_symbol == "00679B").then_some(StockExchangeMarket::OverTheCounter.serial())
}

/// 上櫃代號走櫃買來源，其餘（含查不到市場別的）走證交所來源。
#[tokio::test]
async fn market_routed_fetcher_sends_otc_symbols_to_tpex() {
    let listed = StubFetcher::default();
    let otc = StubFetcher::default();
    let fetcher = MarketRoutedMonthlyQuoteFetcher {
        listed: &listed,
        otc: &otc,
        market_of: stub_market_of,
    };
    let month = date(2019, 3, 1);

    for symbol in ["00679B", "0050", "9999X"] {
        fetcher
            .fetch(symbol, month)
            .await
            .expect("假抓取端不會失敗");
    }

    assert_eq!(otc.requested(), vec![("00679B".to_string(), month)]);
    assert_eq!(
        listed.requested(),
        vec![("0050".to_string(), month), ("9999X".to_string(), month)]
    );
}

/// 市場別取自股票主檔快取；不在快取內的代號回傳 None。
#[test]
fn cached_market_id_reads_the_stock_cache() {
    const SYMBOL: &str = "79951";
    let stock = crate::domain::registry::entity::Stock::register(
        SYMBOL.to_string(),
        "測試上櫃".to_string(),
        StockExchangeMarket::OverTheCounter.serial(),
        1,
    );
    SHARE
        .stocks
        .write()
        .expect("stocks lock")
        .insert(SYMBOL.to_string(), stock);

    assert_eq!(
        cached_market_id(SYMBOL),
        Some(StockExchangeMarket::OverTheCounter.serial())
    );
    SHARE.stocks.write().expect("stocks lock").remove(SYMBOL);
    assert_eq!(cached_market_id(SYMBOL), None);
}
