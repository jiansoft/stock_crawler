use std::collections::HashMap;

use anyhow::{anyhow, Result};
use regex::Regex;
use rust_decimal::Decimal;
use scraper::{Html, Selector};

use crate::{
    crawler::yahoo::HOST,
    util::{http, text},
};

#[derive(Debug, Clone)]
pub struct YahooDividend {
    /// 股票代碼。
    pub stock_symbol: String,
    /// 股利詳情列表，依「發放年度」由新到舊排序（desc）。
    ///
    /// 每個元素為 `(year, details)`：
    /// - `year`：發放年度（使用除息日或除權日推得）
    /// - `details`：該年度的股利明細
    pub dividend: Vec<(i32, Vec<YahooDividendDetail>)>,
}

#[derive(Debug, Clone)]
pub struct YahooDividendDetail {
    /// 發放年度
    pub year: i32,
    /// 股利所屬年度
    pub year_of_dividend: i32,
    /// 季度 Q4 Q3 Q2 Q1 H2 H1
    pub quarter: String,
    /// 現金股利
    pub cash_dividend: Decimal,
    /// 股票股利
    pub stock_dividend: Decimal,
    /// 除息日
    pub ex_dividend_date1: String,
    /// 除權日
    pub ex_dividend_date2: String,
    /// 現金股利發放日
    pub payable_date1: String,
    /// 股票股利發放日
    pub payable_date2: String,
}

impl YahooDividend {
    /// 建立 `YahooDividend` 物件。
    pub fn new(stock_symbol: String) -> Self {
        YahooDividend {
            stock_symbol,
            dividend: vec![],
        }
    }

    /// 依發放年度取得該年度的股利明細。
    ///
    /// # 參數
    ///
    /// - `year`：欲查詢的發放年度
    ///
    /// # 回傳
    ///
    /// - `Some(&Vec<YahooDividendDetail>)`：找到該年度資料
    /// - `None`：找不到該年度資料
    pub fn get_dividend_by_year(&self, year: i32) -> Option<&Vec<YahooDividendDetail>> {
        self.dividend
            .iter()
            .find(|(y, _)| *y == year)
            .map(|(_, details)| details)
    }
}

/// 從 Yahoo 台股頁面抓取指定股票的股利資料。
///
/// 目前會解析以下欄位：
/// - 股利所屬期間（年 / 季，例如 `2024Q4`）
/// - 現金股利
/// - 股票股利
/// - 除息日
/// - 除權日
/// - 現金股利發放日
/// - 股票股利發放日
///
/// # 參數
///
/// * `stock_symbol`: 股票代碼
///
/// # 回傳
///
/// 返回 `Result<YahooDividend>`：
/// - `Ok(YahooDividend)`：抓取與解析成功，且 `dividend` 會依年度由新到舊排序
/// - `Err`：抓取或解析過程發生錯誤
///
/// # 錯誤
///
/// 此函數可能因為網路請求失敗、網頁解析失敗或正規表示式解析失敗等原因導致錯誤。
pub async fn visit(stock_symbol: &str) -> Result<YahooDividend> {
    let url = format!("https://{}/quote/{}/dividend", HOST, stock_symbol);
    let text = http::get(&url, None).await?;
    let document = Html::parse_document(&text);
    //#main-2-QuoteDividend-Proxy > div > section.Mb\(\$m-module\).Mb\(\$mobile-m-module\)--mobile > div.Pos\(r\).Ov\(h\) > div.table-body.Pos\(r\).Bxz\(bb\).W\(100\%\).Ovx\(s\).Ovy\(h\) > div > div > ul > li:nth-child(1) > div
    let selector = match Selector::parse(
        "#main-2-QuoteDividend-Proxy > div > section > div > div > div > div > ul > li",
    ) {
        Ok(selector) => selector,
        Err(why) => {
            return Err(anyhow!("Failed to Selector::parse because: {:?}", why));
        }
    };

    let re = Regex::new(r"(\d+)(Q\d|H\d)?")?;
    let mut dividend_by_year = HashMap::<i32, Vec<YahooDividendDetail>>::new();

    for element in document.select(&selector) {
        let dividend_period = http::element::parse_value(&element, "div > div.Fxg\\(1\\).Fxs\\(1\\).Fxb\\(0\\%\\).Ta\\(end\\).Mend\\(0\\)\\:lc.Mend\\(12px\\).W\\(88px\\).Miw\\(88px\\)");

        if dividend_period.is_none() {
            continue;
        }

        let dividend_date1 = http::element::parse_value(&element, "div > div:nth-child(7)");
        let dividend_date2 = http::element::parse_value(&element, "div > div:nth-child(8)");
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
        let cash_dividend =
            parse_dividend_value(&http::element::parse_value(&element, "div > div:nth-child(3)"));
        let stock_dividend =
            parse_dividend_value(&http::element::parse_value(&element, "div > div:nth-child(4)"));

        let payout_date1 = http::element::parse_value(&element, "div > div:nth-child(9)")
            .unwrap_or_default()
            .replace('/', "-");
        let payout_date2 = http::element::parse_value(&element, "div > div:nth-child(10)")
            .unwrap_or_default()
            .replace('/', "-");
        dividend_by_year
            .entry(year)
            .or_default()
            .push(YahooDividendDetail {
                year,
                year_of_dividend,
                quarter,
                cash_dividend,
                stock_dividend,
                ex_dividend_date1: dividend_date_1,
                ex_dividend_date2: dividend_date_2,
                payable_date1: payout_date1,
                payable_date2: payout_date2,
            });
    }

    let mut e = YahooDividend::new(stock_symbol.to_string());
    e.dividend = dividend_by_year.into_iter().collect();
    e.dividend
        .sort_unstable_by(|(year_a, _), (year_b, _)| year_b.cmp(year_a));

    Ok(e)
}

/// 解析現金股利與股票股利，解析失敗時回傳 0。
fn parse_dividend_value(value: &Option<String>) -> Decimal {
    value
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "-")
        .and_then(|v| text::parse_decimal(v, None).ok())
        .unwrap_or(Decimal::ZERO)
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
                dbg!(&e);
                logging::debug_file_async(format!("e:{:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because {:?}", why));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
    }
}
