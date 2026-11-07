use anyhow::Result;
use async_trait::async_trait;

use crate::infra::crawler::{fbs::HOST, share, share::AnnualProfitFetcher};

/// 富邦證券年度獲利資料來源型別。
pub struct Fbs {}

/// 組出富邦證券年度獲利頁面的網址。
///
/// 抽成純函式讓網址格式可被單元測試鎖定，避免改版時悄悄打錯路徑。
fn build_url(stock_symbol: &str) -> String {
    format!(
        "https://{host}/z/zc/zcdj_{stock_symbol}.djhtm",
        host = HOST,
        stock_symbol = stock_symbol,
    )
}

/// 抓取年度股利資料
pub async fn visit(stock_symbol: &str) -> Result<Vec<share::AnnualProfit>> {
    Ok(share::fetch_annual_profits(&build_url(stock_symbol), stock_symbol).await?)
}

#[async_trait]
impl AnnualProfitFetcher for Fbs {
    async fn visit(stock_symbol: &str) -> Result<Vec<share::AnnualProfit>> {
        visit(stock_symbol).await
    }
}

#[cfg(test)]
mod tests;
