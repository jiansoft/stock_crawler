use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rust_decimal::Decimal;
use scraper::{ElementRef, Html, Selector};

use crate::{
    crawler::{ipify, seeip},
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
    let sales_per_share = text::parse_decimal(tds.get(5)?, None).unwrap_or_else(Decimal::ZERO);

    Some(AnnualProfit {
        stock_symbol: stock_symbol.to_string(),
        earnings_per_share,
        profit_before_tax,
        sales_per_share,
        year,
    })
}

/// 取得對外的 IP
pub async fn get_public_ip() -> Result<String> {
    if let Ok(ip) = ipify::visit().await {
        if !ip.is_empty() {
            return Ok(ip);
        }
    }

    seeip::visit().await
}
