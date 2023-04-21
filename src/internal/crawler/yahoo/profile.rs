use crate::{
    internal::{
        util,
        logging
    }
};
use anyhow::*;
use core::result::Result::Ok;
use regex::Regex;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FinancialStatement {
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
    pub serial: i64,
    /// 年度
    pub year: i32,
}

impl FinancialStatement {
    pub fn new(security_code: String) -> Self {
        FinancialStatement {
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
            serial: 0,
            year: 0,
        }
    }
}

/// 將未下市每股淨值為零的股票試著到yahoo 抓取數據後更新回 stocks表
pub async fn visit(stock_symbol: &str) -> Result<FinancialStatement> {
    let url = format!("https://tw.stock.yahoo.com/quote/{}/profile", stock_symbol);

    logging::info_file_async(format!("visit url:{}", url,));

    let text = util::http::request_get(&url).await?;
    let document = Html::parse_document(text.as_str());
    let selector = match Selector::parse("#main-2-QuoteProfile-Proxy > div > section:nth-child(3)")
    {
        Ok(selector) => selector,
        Err(why) => {
            return Err(anyhow!("Failed to Selector::parse because: {:?}", why));
        }
    };
    let mut e = FinancialStatement::new(stock_symbol.to_string());
    let css_base = "div.table-grid.Mb\\(20px\\).row-fit-half > div:nth-child";

    for element in document.select(&selector) {
        let year_and_quarter =
            util::http::parse::element_value(&element, "div:nth-child(2).D\\(f\\)");
        if let Some(year_and_quarter_text) = year_and_quarter {
            let reg_quarter = Regex::new(r"(?i)q\d")?;
            if let Some(quarter_match) = reg_quarter.find(year_and_quarter_text.as_str()) {
                let year_and_quarter = quarter_match.as_str();
                e.quarter = year_and_quarter.to_uppercase();
                if let Ok(year) = year_and_quarter_text[0..4].parse::<i32>() {
                    e.year = year;
                }
            }
        }

        let fields = [
            (1, &mut e.gross_profit),
            (2, &mut e.return_on_assets),
            (3, &mut e.operating_profit_margin),
            (4, &mut e.return_on_equity),
            (5, &mut e.pre_tax_income),
            (6, &mut e.net_asset_value_per_share),
        ];

        for (css_index, field) in fields {
            *field = element_value_to_decimal(&element, &css_selector(css_base, css_index));
        }

        // 每股稅後淨利
        e.earnings_per_share =
            element_value_to_decimal(&element, "div:nth-child(4) > div:nth-child(3) > div > div");
    }

    Ok(e)
}

fn css_selector(base: &str, child: u32) -> String {
    format!("{}({}) > div > div", base, child)
}

/// 解析完元素的值後將其轉成 decimal
fn element_value_to_decimal(element: &scraper::ElementRef, css_selector: &str) -> Decimal {
    util::http::parse::element_value(element, css_selector)
        .and_then(|v| {
            let value = v.replace(&['元', '%', ' '][..], "");
            Decimal::from_str(&value).ok()
        })
        .unwrap_or(dec!(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::logging;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        logging::info_file_async("開始 visit".to_string());

        match visit("2330").await {
            Ok(e) => {
                println!("{:#?}", e);
            }
            Err(why) => {
                logging::error_file_async(format!("Failed to visit because {:?}", why));
            }
        }

        logging::info_file_async("結束 visit".to_string());
    }
}
