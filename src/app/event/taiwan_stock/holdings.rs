//! # 持股通知共用
//!
//! 持股法說會、主力進出、董監質押等通知都只看「目前持股中的普通股」，
//! 這裡集中取代號與股名，避免各通知各寫一份篩選規則。

use std::collections::BTreeSet;

use anyhow::Result;

use crate::{
    domain::portfolio::repository::PortfolioRepository,
    infra::{cache::SHARE, database::repository::portfolio::PgPortfolioRepository},
};

/// 目前持股（未售出）中的普通股代號，去重並排序。
///
/// # Errors
///
/// 查詢持股失敗時回傳錯誤。
pub(super) async fn common_stock_holdings() -> Result<BTreeSet<String>> {
    let holdings = PgPortfolioRepository::new()
        .fetch_active_holdings(None)
        .await?;
    Ok(common_stocks(
        holdings.into_iter().map(|holding| holding.security_code),
    ))
}

/// 從持股代號中挑出普通股（去重、排序）；持股可能重複（同一檔分次買進）。
pub(super) fn common_stocks(security_codes: impl IntoIterator<Item = String>) -> BTreeSet<String> {
    security_codes
        .into_iter()
        .filter(|symbol| is_common_stock(symbol))
        .collect()
}

/// 是否為普通股：排除 ETF／ETN（`00` 開頭）與特別股、受益證券等含英文字母的代號。
pub(super) fn is_common_stock(stock_symbol: &str) -> bool {
    !stock_symbol.starts_with("00") && stock_symbol.chars().all(|c| c.is_ascii_digit())
}

/// 從股票主檔快取取股名；查不到時回空字串。
pub(super) fn stock_name(stock_symbol: &str) -> String {
    SHARE
        .stocks
        .read()
        .ok()
        .and_then(|stocks| {
            stocks
                .get(stock_symbol)
                .map(|stock| stock.name().to_string())
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
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
}
