use anyhow::Result;
use async_trait::async_trait;

use crate::infra::crawler::{moneydj::HOST, share, share::AnnualProfitFetcher};

/// MoneyDJ 年度獲利資料來源型別。
pub struct MoneyDJ {}
/// 抓取年度股利資料
pub async fn visit(stock_symbol: &str) -> Result<Vec<share::AnnualProfit>> {
    let url = format!(
        "https://{host}/z/zc/zcdj_{stock_symbol}.djhtm",
        host = HOST,
        stock_symbol = stock_symbol,
    );

    Ok(share::fetch_annual_profits(&url, stock_symbol).await?)
}

#[async_trait]
impl AnnualProfitFetcher for MoneyDJ {
    async fn visit(stock_symbol: &str) -> Result<Vec<share::AnnualProfit>> {
        visit(stock_symbol).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 visit");

        match visit("2330").await {
            Ok(e) => {
                dbg!(&e);
                tracing::debug!("moneydj : {:#?}", e);
            }
            Err(why) => {
                tracing::debug!("Failed to visit because {:?}", why);
            }
        }

        tracing::debug!("結束 visit");
    }
}
