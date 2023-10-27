use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::internal::crawler::{
    cmoney::CMoney, cnyes::CnYes, histock::HiStock, megatime::PcHome, yahoo::Yahoo,
};

/// 股市爆料同學會
pub mod cmoney;
/// 玩股網
pub mod cnyes;
/// dds
pub mod free_dns;
/// 股市資訊網
pub mod goodinfo;
/// 嗨投資
pub mod histock;
/// PCHOME
pub mod megatime;
/// 台灣期貨交易所
pub mod taifex;
/// 台灣證券櫃檯買賣中心
pub mod tpex;
/// 台灣證券交易所
pub mod twse;
/// 零股塔
pub mod wespai;
/// 雅虎財經
pub mod yahoo;

#[async_trait]
pub trait StockInfo {
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal>;
}

pub async fn fetch_stock_price_from_remote_site(stock_symbol: &str) -> Result<Decimal> {
    let sites = vec![
        Yahoo::get_stock_price,
        CnYes::get_stock_price,
        PcHome::get_stock_price,
        CMoney::get_stock_price,
        HiStock::get_stock_price,
    ];

    for fetch_func in sites {
        if let Ok(price) = fetch_func(stock_symbol).await {
            return Ok(price);
        }
    }

    Err(anyhow!("Failed to fetch stock price from all sites"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::logging;

    #[tokio::test]
    async fn test_fetch_stock_price_from_remote_site() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_price".to_string());

        match fetch_stock_price_from_remote_site("2330").await {
            Ok(e) => {
                dbg!(e);
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to fetch_price because {:?}", why));
            }
        }

        logging::debug_file_async("結束 fetch_price".to_string());
    }
}
