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
pub(crate) async fn common_stock_holdings() -> Result<BTreeSet<String>> {
    let holdings = PgPortfolioRepository::new()
        .fetch_active_holdings(None)
        .await?;
    Ok(common_stocks(
        holdings.into_iter().map(|holding| holding.security_code),
    ))
}

/// 目前持股（未售出）的全部代號（含 ETF），去重並排序。
///
/// # Errors
///
/// 查詢持股失敗時回傳錯誤。
pub(crate) async fn active_holding_symbols() -> Result<BTreeSet<String>> {
    let holdings = PgPortfolioRepository::new()
        .fetch_active_holdings(None)
        .await?;
    Ok(holdings
        .into_iter()
        .map(|holding| holding.security_code)
        .collect())
}

/// 目前持股（未售出）每筆的代號（含 ETF）與股數；同一代號分次買進、不同會員會有多筆。
///
/// # Errors
///
/// 查詢持股失敗時回傳錯誤。
pub(crate) async fn active_holding_quantities() -> Result<Vec<(String, i64)>> {
    let holdings = PgPortfolioRepository::new()
        .fetch_active_holdings(None)
        .await?;
    Ok(holdings
        .into_iter()
        .map(|holding| (holding.security_code, holding.share_quantity))
        .collect())
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
pub(crate) fn stock_name(stock_symbol: &str) -> String {
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
mod tests;
