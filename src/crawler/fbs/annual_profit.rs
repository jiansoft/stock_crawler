use anyhow::Result;
use async_trait::async_trait;

use crate::crawler::{fbs::HOST, share, share::AnnualProfitFetcher};

pub struct Fbs {}

/// 抓取年度股利資料
pub async fn visit(stock_symbol: &str) -> Result<Vec<share::AnnualProfit>> {
    let url = format!(
        "https://{host}/z/zc/zcdj_{stock_symbol}.djhtm",
        host = HOST,
        stock_symbol = stock_symbol,
    );

    share::fetch_annual_profits(&url, stock_symbol).await
}

#[async_trait]
impl AnnualProfitFetcher for Fbs {
    async fn visit(stock_symbol: &str) -> Result<Vec<share::AnnualProfit>> {
        visit(stock_symbol).await
    }
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 visit".to_string());

        match visit("2838").await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("fbs : {:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because {:?}", why));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
    }
}
