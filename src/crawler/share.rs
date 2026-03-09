use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rust_decimal::Decimal;
use scraper::{ElementRef, Html, Selector};

use crate::crawler::{bigdatacloud, myip};
use crate::{
    crawler::{ipconfig, ipify, ipinfo, seeip},
    util::{self, map::Keyable, text},
};

/// 年度財報
#[derive(Debug, Clone, PartialEq)]
pub struct AnnualProfit {
    /// Security code
    pub stock_symbol: String,
    /// 財報年度 (Year)
    pub year: i32,
    /// 每股營收
    pub sales_per_share: Decimal,
    /// 每股稅後淨利
    pub earnings_per_share: Decimal,
    /// 每股稅前淨利
    pub profit_before_tax: Decimal,
}

impl AnnualProfit {
    pub fn new(stock_symbol: String) -> Self {
        Self {
            stock_symbol,
            year: 0,
            sales_per_share: Default::default(),
            earnings_per_share: Default::default(),
            profit_before_tax: Default::default(),
        }
    }
}

impl Keyable for AnnualProfit {
    fn key(&self) -> String {
        format!("{}-{}", self.stock_symbol, self.year)
    }

    fn key_with_prefix(&self) -> String {
        format!("AnnualProfit:{}", self.key())
    }
}

#[async_trait]
pub trait AnnualProfitFetcher {
    async fn visit(stock_symbol: &str) -> Result<Vec<AnnualProfit>>;
}

pub(super) async fn fetch_annual_profits(
    url: &str,
    stock_symbol: &str,
) -> Result<Vec<AnnualProfit>> {
    let text = util::http::get(url, None).await?;
    let document = Html::parse_document(&text);
    let selector = Selector::parse("#oMainTable > tbody > tr:nth-child(n+4)")
        .map_err(|why| anyhow!("Failed to Selector::parse because: {:?}", why))?;
    let mut result: Vec<AnnualProfit> = Vec::with_capacity(24);

    for node in document.select(&selector) {
        if let Some(ap) = parse_annual_profit(node, stock_symbol) {
            result.push(ap);
        }
    }

    Ok(result)
}

fn parse_annual_profit(node: ElementRef, stock_symbol: &str) -> Option<AnnualProfit> {
    let tds: Vec<&str> = node.text().map(str::trim).collect();

    if tds.len() < 8 {
        return None;
    }

    let year = text::parse_i32(tds.first()?, None)
        .ok()
        .map(util::datetime::roc_year_to_gregorian_year)?;
    let earnings_per_share = text::parse_decimal(tds.get(7)?, None).ok()?;
    let profit_before_tax = text::parse_decimal(tds.get(6)?, None).unwrap_or(Decimal::ZERO);
    let sales_per_share = text::parse_decimal(tds.get(5)?, None).unwrap_or(Decimal::ZERO);

    Some(AnnualProfit {
        stock_symbol: stock_symbol.to_string(),
        earnings_per_share,
        profit_before_tax,
        sales_per_share,
        year,
    })
}

type IpFetchFn = dyn Fn() -> Pin<Box<dyn Future<Output = Result<String>> + Send>> + Sync;

/// 全域 IP 查詢游標，用於順序輪詢不同的檢測服務。
static IP_INDEX: AtomicUsize = AtomicUsize::new(0);

/// 獲取系統對外的公網 IP 地址。
///
/// 此函式透過多個第三方 IP 檢測服務進行輪詢，以確保在單一服務失效時仍能取得 IP。
/// 為了平衡負載並避免單一服務請求過於頻繁，採用順序輪詢 (Round-robin) 機制切換不同站點。
///
/// # 支援的服務站點
/// - `ipify.org`
/// - `ipconfig.io`
/// - `ipinfo.io`
/// - `seeip.org`
/// - `myip.com`
/// - `bigdatacloud.com`
///
/// # 回傳值
/// - `Ok(String)`: 成功取得的公網 IP 字串。
/// - `Ok("")`: 若所有站點都嘗試失敗，則回傳空字串（不拋出錯誤，由呼叫端決定後續行為）。
/// - `Err`: 發生嚴重的系統層級錯誤。
pub async fn get_public_ip() -> Result<String> {
    let sites: [&IpFetchFn; 6] = [
        &|| Box::pin(ipify::visit()),
        &|| Box::pin(ipconfig::visit()),
        &|| Box::pin(ipinfo::visit()),
        &|| Box::pin(seeip::visit()),
        &|| Box::pin(myip::visit()),
        &|| Box::pin(bigdatacloud::visit()),
    ];

    let site_len = sites.len();

    for _ in 0..site_len {
        // 取得並遞增游標，實現跨呼叫的順序輪詢
        let current_index = IP_INDEX.fetch_add(1, Ordering::SeqCst) % site_len;
        let site = sites[current_index];

        if let Ok(ip) = site().await {
            if !ip.is_empty() {
                return Ok(ip);
            }
        }
    }

    Ok(String::from(""))
}
