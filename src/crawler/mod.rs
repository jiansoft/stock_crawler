use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::{
    crawler::{
        cmoney::CMoney, cnyes::CnYes, histock::HiStock, megatime::PcHome, nstock::NStock,
        yahoo::Yahoo,
    },
    declare,
};

pub mod afraid;
/// 臺灣銀行
pub mod bank_of_taiwan;
/// 理財寶-股市爆料同學會
pub mod cmoney;
/// 鉅亨網
pub mod cnyes;
pub mod dynu;
/// 富邦證券
pub mod fbs;
/// 股市資訊網
pub mod goodinfo;
/// 嗨投資
pub mod histock;
pub mod ipify;
/// PCHOME
pub mod megatime;
/// 嘉實資訊-理財網
pub mod moneydj;
pub mod noip;
/// 恩投資
pub mod nstock;
pub mod seeip;
/// 共用 元大證券、嘉實資訊-理財網、富邦證券
pub(super) mod share;
/// 台灣期貨交易所
pub mod taifex;
/// 台灣證券櫃檯買賣中心
pub mod tpex;
/// 台灣證券交易所
pub mod twse;
/// 撿股讚
pub mod wespai;
/// 雅虎財經
pub mod yahoo;
/// 元大證券
pub mod yuanta;

#[async_trait]
pub trait StockInfo {
    async fn get_stock_price(stock_symbol: &str) -> Result<Decimal>;
    async fn get_stock_quotes(stock_symbol: &str) -> Result<declare::StockQuotes>;
}

/// 標記採集站點的遊標，每採集一次遊標就會+1，分別對應6個站點，每個站點都輪過一次時就會歸零從頭開始
static INDEX: AtomicUsize = AtomicUsize::new(0);

/// 取得股票的目前的報價
pub async fn fetch_stock_price_from_remote_site(stock_symbol: &str) -> Result<Decimal> {
    let sites = [
        Yahoo::get_stock_price,
        NStock::get_stock_price,
        CnYes::get_stock_price,
        PcHome::get_stock_price,
        CMoney::get_stock_price,
        HiStock::get_stock_price,
    ];
    let site_len = sites.len();

    for _ in 0..site_len {
        let index = INDEX.fetch_add(1, Ordering::SeqCst) % site_len;
        let current_site = index % site_len;
        let r = sites[current_site](stock_symbol).await;

        if r.is_ok() {
            return r;
        }
    }

    Err(anyhow!(
        "Failed to fetch stock price({}) from all sites",
        stock_symbol
    ))
}

/// 取得股票目前的報價含漲跌、漲幅
pub async fn fetch_stock_quotes_from_remote_site(
    stock_symbol: &str,
) -> Result<declare::StockQuotes> {
    let sites = [
        NStock::get_stock_quotes,
        Yahoo::get_stock_quotes,
        CnYes::get_stock_quotes,
        PcHome::get_stock_quotes,
        CMoney::get_stock_quotes,
        HiStock::get_stock_quotes,
    ];
    let site_len = sites.len();

    for _ in 0..site_len {
        let index = INDEX.fetch_add(1, Ordering::SeqCst) % site_len;
        let current_site = index % site_len;
        let r = sites[current_site](stock_symbol).await;

        if r.is_ok() {
            return r;
        }
    }

    Err(anyhow!(
        "Failed to fetch stock quotes({}) from all sites",
        stock_symbol
    ))
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[tokio::test]
    async fn test_fetch_stock_price_from_remote_site() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_price".to_string());

        let sites = [
            "2330", "1101", "1232", "1303", "1326", "3008", "9941", "2912",
        ];

        for site in sites {
            match fetch_stock_price_from_remote_site(site).await {
                Ok(e) => {
                    //dbg!(e);
                    println!("{}:{}", site, e);
                }
                Err(why) => {
                    logging::debug_file_async(format!("Failed to fetch_price because {:?}", why));
                }
            }
        }

        logging::debug_file_async("結束 fetch_price".to_string());
    }

    #[tokio::test]
    async fn test_fetch_stock_quotes_from_remote_site() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_stock_quotes_from_remote_site".to_string());

        let sites = [
            "2330", "1101", "1232", "1303", "1326", "3008", "9941", "2912",
        ];

        for site in sites {
            match fetch_stock_quotes_from_remote_site(site).await {
                Ok(e) => {
                    //dbg!(e);
                    println!("{}:{:?}", site, e);
                }
                Err(why) => {
                    logging::debug_file_async(format!("Failed to fetch_price because {:?}", why));
                }
            }
        }

        logging::debug_file_async("結束 fetch_stock_quotes_from_remote_site".to_string());
    }
}
