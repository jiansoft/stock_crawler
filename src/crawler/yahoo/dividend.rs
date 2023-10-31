use anyhow::{anyhow, Result};
use hashbrown::HashMap;
use regex::Regex;
use scraper::{Html, Selector};

use crate::{crawler::yahoo::HOST, util::http};

#[derive(Debug, Clone)]
pub struct YahooDividend {
    /// 股票代碼。
    pub stock_symbol: String,
    /// 股利詳情的對應表，鍵為年份，值為該年份的股利詳情列表。
    pub dividend: HashMap<i32, Vec<YahooDividendDetail>>,
}

#[derive(Debug, Clone)]
pub struct YahooDividendDetail {
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

impl YahooDividendDetail {
    pub fn new(
        year: i32,
        year_of_dividend: i32,
        quarter: String,
        ex_dividend_date1: String,
        ex_dividend_date2: String,
        payable_date1: String,
        payable_date2: String,
    ) -> Self {
        YahooDividendDetail {
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

impl YahooDividend {
    pub fn new(stock_symbol: String) -> Self {
        YahooDividend {
            stock_symbol,
            dividend: Default::default(),
        }
    }
}

/// 從 Yahoo 網站抓取指定股票代碼的股利除息日、除權日、現金股利發放日、股票股利發放日等資訊。
///
/// # 參數
///
/// * `stock_symbol`: 股票代碼
///
/// # 回傳
///
/// 返回一個結果，該結果為 `Result<Dividend>` 型態，當抓取成功時返回 `Ok(Dividend)`，
/// `Dividend` 結構體包含了股票代碼與該股票的所有股利資訊。
/// 若在抓取過程中發生錯誤，則返回 `Err`。
///
/// # 錯誤
///
/// 此函數可能因為網路請求失敗、網頁解析失敗或正規表示式解析失敗等原因導致錯誤。
pub async fn visit(stock_symbol: &str) -> Result<YahooDividend> {
    let url = format!("https://{}/quote/{}/dividend", HOST, stock_symbol);
    let text = http::get(&url, None).await?;
    let document = Html::parse_document(text.as_str());
    let selector = match Selector::parse(
        "#main-2-QuoteDividend-Proxy > div > section > div > div > div > div > ul > li",
    ) {
        Ok(selector) => selector,
        Err(why) => {
            return Err(anyhow!("Failed to Selector::parse because: {:?}", why));
        }
    };

    let re = Regex::new(r"(\d+)(Q\d|H\d)?")?;
    let mut e = YahooDividend::new(stock_symbol.to_string());

    for element in document.select(&selector) {
        let dividend_period = http::element::parse_value(&element, "div > div > div");
        if dividend_period.is_none() {
            continue;
        }

        let dividend_date1 = http::element::parse_value(&element, "div > div:nth-child(5)");
        let dividend_date2 = http::element::parse_value(&element, "div > div:nth-child(6)");
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

        let payout_date1 = http::element::parse_value(&element, "div > div:nth-child(7)")
            .unwrap_or_default()
            .replace('/', "-");
        let payout_date2 = http::element::parse_value(&element, "div > div:nth-child(8)")
            .unwrap_or_default()
            .replace('/', "-");
        e.dividend
            .entry(year)
            .or_insert_with(Vec::new)
            .push(YahooDividendDetail::new(
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

/// 解析日期，並將年份設定到參數 year 中。
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

/// 解析股利期間，返回股利所屬的年份和季度。
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
    use crate::logging;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 visit".to_string());

        match visit("2330").await {
            Ok(e) => {
                logging::debug_file_async(format!("e:{:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because {:?}", why));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
    }
}
