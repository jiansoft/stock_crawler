use std::result::Result::Ok;

use anyhow::*;
use regex::Regex;
use reqwest::header::HeaderMap;
use rust_decimal::Decimal;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

use crate::{internal::crawler::wespai::HOST, util::http, util::http::element};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Profit {
    /// 季度 Q4 Q3 Q2 Q1
    pub quarter: String,
    pub security_code: String,
    /// 營業毛利率
    pub gross_profit: Decimal,
    /// 營業利益率
    pub operating_profit_margin: Decimal,
    /// 稅前淨利率
    pub pre_tax_income: Decimal,
    /// 稅後淨利率
    pub net_income: Decimal,
    /// 每股淨值
    pub net_asset_value_per_share: Decimal,
    /// 每股營收
    pub sales_per_share: Decimal,
    /// 每股稅後淨利
    pub earnings_per_share: Decimal,
    /// 每股稅前淨利
    pub profit_before_tax: Decimal,
    /// 股東權益報酬率
    pub return_on_equity: Decimal,
    /// 資產報酬率
    pub return_on_assets: Decimal,
    /// 年度
    pub year: i32,
}

impl Profit {
    pub fn new(year: i32, security_code: String) -> Self {
        Profit {
            quarter: "".to_string(),
            security_code,
            gross_profit: Default::default(),
            operating_profit_margin: Default::default(),
            pre_tax_income: Default::default(),
            net_income: Default::default(),
            net_asset_value_per_share: Default::default(),
            sales_per_share: Default::default(),
            earnings_per_share: Default::default(),
            profit_before_tax: Default::default(),
            return_on_equity: Default::default(),
            return_on_assets: Default::default(),
            year,
        }
    }
}

/// 抓取年報
pub async fn visit() -> Result<Vec<Profit>> {
    let url = format!("https://stock.{}/profit", HOST);
    let ua = http::user_agent::gen_random_ua();
    let mut headers = HeaderMap::new();

    headers.insert("Referer", url.parse()?);
    headers.insert("User-Agent", ua.parse()?);
    headers.insert("content-length", "0".parse()?);

    let text = http::get(&url, Some(headers)).await?;
    let document = Html::parse_document(text.as_str());
    let selector = match Selector::parse("body > h1 > a") {
        Ok(selector) => selector,
        Err(why) => {
            return Err(anyhow!("Failed to Selector::parse because: {:?}", why));
        }
    };
    let year = match document.select(&selector).next() {
        None => {
            return Err(anyhow!("Failed to select .next()"));
        }
        Some(year) => year,
    };
    let year = match year.text().next() {
        None => {
            return Err(anyhow!("Failed to parse year raw({:?})", year));
        }
        Some(year) => year,
    };
    let re = Regex::new(r"\d{4}")?;
    let mut profit_year = 0;
    if let Some(caps) = re.captures(year) {
        if let Some(q) = caps.get(0) {
            profit_year = q.as_str().parse::<i32>()?
        }
    }

    if profit_year == 0 {
        return Err(anyhow!("profit_year is zero"));
    }

    let selector = match Selector::parse("#example > tbody > tr") {
        Ok(selector) => selector,
        Err(why) => {
            return Err(anyhow!("Failed to Selector::parse because: {:?}", why));
        }
    };
    let mut profits = Vec::with_capacity(2048);

    for element in document.select(&selector) {
        //let tds: Vec<&str> = element.text().collect();
        //println!("tds:{:#?}",tds);
        let security_code = match element::parse_value(&element, "td:nth-child(1)") {
            None => continue,
            Some(security_code) => security_code,
        };

        let mut p = Profit::new(profit_year, security_code);
        //grossProfit := s.Find(fmt.Sprintf("td:nth-child(%d)", 3+jumpColumnCount)).Text()
        p.gross_profit = element::parse_to_decimal(&element, "td:nth-child(4)");
        //	operatingProfitMargin := s.Find(fmt.Sprintf("td:nth-child(%d)", 4+jumpColumnCount)).Text()
        p.operating_profit_margin = element::parse_to_decimal(&element, "td:nth-child(5)");
        //preTaxIncome := s.Find(fmt.Sprintf("td:nth-child(%d)", 5+jumpColumnCount)).Text()
        p.pre_tax_income = element::parse_to_decimal(&element, "td:nth-child(6)");
        //netIncome := s.Find(fmt.Sprintf("td:nth-child(%d)", 6+jumpColumnCount)).Text()
        p.net_income = element::parse_to_decimal(&element, "td:nth-child(7)");
        //netAssetValuePerShare := s.Find(fmt.Sprintf("td:nth-child(%d)", 7+jumpColumnCount)).Text()
        p.net_asset_value_per_share = element::parse_to_decimal(&element, "td:nth-child(8)");
        //salesPerShare := s.Find(fmt.Sprintf("td:nth-child(%d)", 8+jumpColumnCount)).Text()
        p.sales_per_share = element::parse_to_decimal(&element, "td:nth-child(9)");
        //earningsPerShare := s.Find(fmt.Sprintf("td:nth-child(%d)", 13+jumpColumnCount)).Text()
        p.earnings_per_share = element::parse_to_decimal(&element, "td:nth-child(14)");
        //profitBeforeTax := s.Find(fmt.Sprintf("td:nth-child(%d)", 10+jumpColumnCount)).Text()
        p.profit_before_tax = element::parse_to_decimal(&element, "td:nth-child(11)");
        //returnOnEquity := s.Find(fmt.Sprintf("td:nth-child(%d)", 11+jumpColumnCount)).Text()
        p.return_on_equity = element::parse_to_decimal(&element, "td:nth-child(12)");
        //returnOnAssets := s.Find(fmt.Sprintf("td:nth-child(%d)", 12+jumpColumnCount)).Text()
        p.return_on_assets = element::parse_to_decimal(&element, "td:nth-child(13)");
        //logging::debug_file_async(format!("p:{:#?}", p));
        profits.push(p);
    }

    Ok(profits)
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 visit".to_string());

        match visit().await {
            Ok(e) => {
                logging::debug_file_async(format!("{:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because {:?}", why));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
    }
}
