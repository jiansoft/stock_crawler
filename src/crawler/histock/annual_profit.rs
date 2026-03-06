//! # HiStock 年度財報採集
//!
//! 此模組透過 HiStock 的「每股盈餘」頁面抓取歷年年度 EPS。
//!
//! 目前解析策略：
//! - 從頁面文字節點中找出 `季別/年度` 標頭列
//! - 再找出對應的 `總計` 列
//! - 以 `總計` 列作為各年度 EPS
//!
//! 由於此頁面未直接提供 `sales_per_share` 與 `profit_before_tax`，
//! 目前這兩個欄位暫時以 `0` 回填。

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rust_decimal::Decimal;
use scraper::Html;

use crate::{
    crawler::{
        histock::HOST,
        share::{self, AnnualProfitFetcher},
    },
    util::{self, text},
};

/// HiStock 年度財報抓取器。
pub struct HiStockAnnualProfit {}

fn is_year_token(text: &str) -> bool {
    let normalized = text.trim_end_matches('-');
    normalized.len() == 4 && normalized.chars().all(|ch| ch.is_ascii_digit())
}

fn parse_year(text: &str) -> Result<i32> {
    text.trim_end_matches('-')
        .parse::<i32>()
        .map_err(|why| anyhow!("Failed to parse year '{}' because {:?}", text, why))
}

fn parse_eps(text: &str) -> Result<Decimal> {
    if text.trim() == "--" {
        return Ok(Decimal::ZERO);
    }

    text::parse_decimal(text, None)
}

fn parse_annual_profit_from_text_nodes(
    stock_symbol: &str,
    texts: &[String],
) -> Result<Vec<share::AnnualProfit>> {
    let header_idx = texts
        .iter()
        .position(|text| text == "季別/年度")
        .ok_or_else(|| anyhow!("Failed to find HiStock annual header row"))?;
    let total_idx = texts
        .iter()
        .position(|text| text == "總計")
        .ok_or_else(|| anyhow!("Failed to find HiStock annual total row"))?;

    let mut years = Vec::new();
    for text in texts.iter().skip(header_idx + 1) {
        if is_year_token(text) {
            years.push(parse_year(text)?);
            continue;
        }

        if !years.is_empty() {
            break;
        }
    }

    if years.is_empty() {
        return Err(anyhow!("Failed to parse HiStock annual year columns"));
    }

    let mut annual_profits = Vec::with_capacity(years.len());
    for (year, eps_text) in years.into_iter().zip(texts.iter().skip(total_idx + 1)) {
        let earnings_per_share = parse_eps(eps_text)?;
        annual_profits.push(share::AnnualProfit {
            stock_symbol: stock_symbol.to_string(),
            year,
            sales_per_share: Decimal::ZERO,
            earnings_per_share,
            profit_before_tax: Decimal::ZERO,
        });
    }

    Ok(annual_profits)
}

/// 抓取指定股票的年度財報資料。
///
/// 目前以「每股盈餘」頁中的 `總計` 列作為年度 EPS。
pub async fn visit(stock_symbol: &str) -> Result<Vec<share::AnnualProfit>> {
    let url = format!(
        "https://{host}/stock/{stock_symbol}/%E6%AF%8F%E8%82%A1%E7%9B%88%E9%A4%98",
        host = HOST,
        stock_symbol = stock_symbol
    );
    let html = util::http::get(&url, None).await?;
    let document = Html::parse_document(&html);
    let texts = document
        .root_element()
        .text()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    parse_annual_profit_from_text_nodes(stock_symbol, &texts)
}

#[async_trait]
impl AnnualProfitFetcher for HiStockAnnualProfit {
    async fn visit(stock_symbol: &str) -> Result<Vec<share::AnnualProfit>> {
        visit(stock_symbol).await
    }
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[test]
    fn test_parse_annual_profit_from_text_nodes() {
        let texts = vec![
            "其他".to_string(),
            "季別/年度".to_string(),
            "2024".to_string(),
            "2023".to_string(),
            "2022".to_string(),
            "Q4".to_string(),
            "總計".to_string(),
            "1.16".to_string(),
            "3.25".to_string(),
            "--".to_string(),
        ];

        let result = parse_annual_profit_from_text_nodes("2838", &texts).unwrap();

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].year, 2024);
        assert_eq!(
            result[0].earnings_per_share,
            Decimal::from_str_exact("1.16").unwrap()
        );
        assert_eq!(result[1].year, 2023);
        assert_eq!(
            result[1].earnings_per_share,
            Decimal::from_str_exact("3.25").unwrap()
        );
        assert_eq!(result[2].year, 2022);
        assert_eq!(result[2].earnings_per_share, Decimal::ZERO);
    }

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 histock::annual_profit::visit".to_string());

        match visit("2330").await {
            Ok(result) => {
                dbg!(&result);
                logging::debug_file_async(format!("histock : {:#?}", result));
            }
            Err(why) => logging::debug_file_async(format!(
                "Failed to histock::annual_profit::visit because {:?}",
                why
            )),
        }

        logging::debug_file_async("結束 histock::annual_profit::visit".to_string());
    }
}
