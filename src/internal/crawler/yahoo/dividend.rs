use crate::internal::crawler::yahoo::HOST;
use crate::internal::util::http;
use crate::internal::{logging, util};
use anyhow::*;
use core::result::Result::Ok;
use regex::Regex;
use scraper::{Html, Selector};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Dividend {
    pub stock_symbol: String,
    pub dividend: HashMap<i32, Vec<Detail>>,
}

#[derive(Debug, Clone)]
pub struct Detail {
    /// 發放年度
    pub year: i32,
    /// 股利所屬年度
    pub year_of_dividend: i32,
    /// 季度 Q4 Q3 Q2 Q1 H2 H1
    pub quarter: String,
    /// 除息日
    pub ex_dividend_date1: String,
    /// 除權日
    pub ex_dividend_date2: String,
    /// 現金股利發放日
    pub payable_date1: String,
    /// 股票股利發放日
    pub payable_date2: String,
}

impl Detail {
    pub fn new(
        year: i32,
        year_of_dividend: i32,
        quarter: String,
        ex_dividend_date1: String,
        ex_dividend_date2: String,
        payable_date1: String,
        payable_date2: String,
    ) -> Self {
        Detail {
            year,
            year_of_dividend,
            quarter,
            ex_dividend_date1,
            ex_dividend_date2,
            payable_date1,
            payable_date2,
        }
    }
}

impl Dividend {
    pub fn new(stock_symbol: String) -> Self {
        Dividend {
            stock_symbol,
            dividend: Default::default(),
        }
    }
}

/// 從 yahoo 抓取股利的除息日 除權日 現金股利發放日 股票股利發放日 等數據
pub async fn visit(stock_symbol: &str) -> Result<Dividend> {
    let url = format!("https://{}/quote/{}/dividend", HOST, stock_symbol);

    logging::info_file_async(format!("visit url:{}", url,));

    let text = util::http::request_get(&url, None).await?;
    let document = Html::parse_document(text.as_str());
    let selector = match Selector::parse(
        "#main-2-QuoteDividend-Proxy > div > section > div > div > div > div > ul > li",
    ) {
        Ok(selector) => selector,
        Err(why) => {
            return Err(anyhow!("Failed to Selector::parse because: {:?}", why));
        }
    };

    let re = Regex::new(r"(\d+)(Q|H\d)?")?;
    let mut e = Dividend::new(stock_symbol.to_string());

    for element in document.select(&selector) {
        let dividend_period = http::parse::element_value(&element, "div > div > div");
        if dividend_period.is_none() {
            continue;
        }

        let dividend_date1 = http::parse::element_value(&element, "div > div:nth-child(5)");
        let dividend_date2 = http::parse::element_value(&element, "div > div:nth-child(6)");
        if dividend_date1.is_none() && dividend_date2.is_none() {
            continue;
        }

        //使用除息日或除權日當作發放年度
        let mut year = 0;
        //除息日
        let dividend_date_1 = parse_date(&dividend_date1, &mut year);
        //除權日
        let dividend_date_2 = parse_date(&dividend_date2, &mut year);
        // 若除息日或除權日能是尚未公佈則 year 會是 0
        if year == 0 {
            continue;
        }

        //股利所屬期間
        let (year_of_dividend, quarter) = parse_period(&dividend_period, &re)?;

        let payout_date1 = http::parse::element_value(&element, "div > div:nth-child(7)")
            .unwrap_or_default()
            .replace('/', "-");
        let payout_date2 = http::parse::element_value(&element, "div > div:nth-child(8)")
            .unwrap_or_default()
            .replace('/', "-");
        e.dividend
            .entry(year)
            .or_insert_with(Vec::new)
            .push(Detail::new(
                year,
                year_of_dividend,
                quarter,
                dividend_date_1,
                dividend_date_2,
                payout_date1,
                payout_date2,
            ));
    }

    Ok(e)
}

fn parse_date(date: &Option<String>, year: &mut i32) -> String {
    match date {
        Some(date_str) if !date_str.is_empty() && !date_str.contains('-') => {
            if *year == 0 {
                if let Some(year_str) = date_str.split('/').next() {
                    if let Ok(y) = year_str.parse::<i32>() {
                        *year = y;
                    }
                }
            }

            date_str.replace('/', "-")
        }
        _ => String::from("-"),
    }
}

fn parse_period(period: &Option<String>, re: &Regex) -> Result<(i32, String)> {
    let mut year_of_dividend = 0;
    let mut quarter = String::from("");
    if let Some(period) = period {
        if let Some(caps) = re.captures(period) {
            if let Some(q) = caps.get(1) {
                year_of_dividend = q.as_str().parse::<i32>()?
            }
            if let Some(q) = caps.get(2) {
                quarter = q.as_str().to_string();
            }
        }
    }

    Ok((year_of_dividend, quarter))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::logging;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 visit".to_string());

        match visit("5287").await {
            Ok(e) => {
                println!("{:#?}", e);
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because {:?}", why));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
    }
}
