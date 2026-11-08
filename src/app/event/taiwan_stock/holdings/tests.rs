//! 持股通知共用的測試。

use chrono::Local;

use super::*;

/// 持股可能重複（同一檔分次買進），挑出來的代號去重並排序。
#[test]
fn common_stocks_deduplicates_and_skips_etfs() {
    let symbols = common_stocks(
        ["2330", "0050", "2330", "2753", "2887G"]
            .into_iter()
            .map(str::to_string),
    );
    assert_eq!(
        symbols.into_iter().collect::<Vec<_>>(),
        vec!["2330".to_string(), "2753".to_string()]
    );
}

#[test]
fn is_common_stock_skips_etfs_and_preferred_shares() {
    assert!(is_common_stock("2330"));
    assert!(is_common_stock("6505"));
    for symbol in ["0050", "00878", "2887G", "2887Z1"] {
        assert!(!is_common_stock(symbol), "{symbol}");
    }
}

/// 全部持股代號包含普通股持股（另含 ETF 等）；查詢失敗時跳過。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn active_holding_symbols_cover_common_stocks() {
    dotenvy::dotenv().ok();
    let (Ok(all), Ok(common)) = (
        active_holding_symbols().await,
        common_stock_holdings().await,
    ) else {
        println!("跳過 active_holding_symbols_cover_common_stocks：無資料庫連接");
        return;
    };
    assert!(common.is_subset(&all), "{common:?} ⊄ {all:?}");
    // 股數明細涵蓋同一批代號。
    let quantities = active_holding_quantities()
        .await
        .expect("同一個查詢剛才成功過");
    let symbols: BTreeSet<String> = quantities.into_iter().map(|(symbol, _)| symbol).collect();
    assert_eq!(symbols, all);
}

/// 股名取自股票主檔快取；查不到時為空字串。
#[test]
fn stock_name_reads_the_registry_cache() {
    let symbol = "79988";
    SHARE.stocks.write().expect("stocks 快取可寫入").insert(
        symbol.to_string(),
        crate::domain::registry::entity::Stock::reconstitute(
            symbol.to_string(),
            "測試持股".to_string(),
            false,
            rust_decimal::Decimal::ZERO,
            rust_decimal::Decimal::ZERO,
            rust_decimal::Decimal::ZERO,
            Local::now(),
            2,
            24,
            0,
            0,
            rust_decimal::Decimal::ZERO,
        ),
    );
    assert_eq!(stock_name(symbol), "測試持股");
    SHARE
        .stocks
        .write()
        .expect("stocks 快取可寫入")
        .remove(symbol);
    assert_eq!(stock_name(symbol), "");
}
