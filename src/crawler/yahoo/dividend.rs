//! # Yahoo 股利政策採集器
//!
//! 此模組負責從 Yahoo 財經抓取股票的歷年股利發放明細。
//! 資料包含現金股利、股票股利、除息/除權日以及實際發放日。
//!
//! ## 資料結構
//!
//! - `YahooDividend`：按「年度」聚合的股利列表體。
//! - `YahooDividendDetail`：單次股利發放的詳細資訊（如 2024Q1 季配息）。
//!
//! ## 解析邏輯
//!
//! - **年度判定**：優先以「除息日」或「除權日」的年份作為發放年度。
//! - **格式化**：自動將網頁上的日期斜線 (`/`) 轉換為標準橫線 (`-`)。
//! - **效能**：使用 `Lazy` 靜態化正則與選擇器，並在內部使用 `HashMap` 進行年度聚合後再排序輸出。

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use rust_decimal::Decimal;
use scraper::{Html, Selector};

use crate::{
    crawler::yahoo::HOST,
    util::{http, text},
};

/// 用於解析股利所屬期間（如 2024Q4）的正則表達式
static REG_PERIOD: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(\d{4})(Q\d|H\d)?").expect("Failed to compile dividend period regex")
});

/// 股利列表明細行的選擇器
static LIST_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse("#main-2-QuoteDividend-Proxy ul > li")
        .expect("Failed to parse dividend list selector")
});

/// 股票股利資料集合體
#[derive(Debug, Clone)]
pub struct YahooDividend {
    /// 股票代碼
    pub stock_symbol: String,
    /// 股利詳情列表，依「發放年度」由新到舊排序（desc）。
    ///
    /// 每個元素為 `(year, details)`：
    /// - `year`：發放年度
    /// - `details`：該年度內所有的配息記錄（例如季配息會有 4 筆）
    pub dividend: Vec<(i32, Vec<YahooDividendDetail>)>,
}

/// 單筆股利明細資訊
#[derive(Debug, Clone)]
pub struct YahooDividendDetail {
    /// 發放年度 (西元)
    pub year: i32,
    /// 股利所屬年度 (西元)
    pub year_of_dividend: i32,
    /// 季度/半年資訊 (例如: "Q4", "H1", "年")
    pub quarter: String,
    /// 現金股利 (元)
    pub cash_dividend: Decimal,
    /// 股票股利 (元)
    pub stock_dividend: Decimal,
    /// 除息日 (格式: YYYY-MM-DD)
    pub ex_dividend_date1: String,
    /// 除權日 (格式: YYYY-MM-DD)
    pub ex_dividend_date2: String,
    /// 現金股利發放日 (格式: YYYY-MM-DD)
    pub payable_date1: String,
    /// 股票股利發放日 (格式: YYYY-MM-DD)
    pub payable_date2: String,
}

impl YahooDividend {
    /// 建立新的 `YahooDividend` 實例。
    pub fn new(stock_symbol: String) -> Self {
        YahooDividend {
            stock_symbol,
            dividend: vec![],
        }
    }

    /// 依發放年度取得該年度的股利明細列表。
    pub fn get_dividend_by_year(&self, year: i32) -> Option<&Vec<YahooDividendDetail>> {
        self.dividend
            .iter()
            .find(|(y, _)| *y == year)
            .map(|(_, details)| details)
    }
}

/// 從 Yahoo 台股頁面抓取指定股票的股利資料。
///
/// # 參數
/// * `stock_symbol` - 股票代碼 (例如: "2330")
///
/// # 實作細節
/// 遍歷股利列表表格，提取各項日期與金額。若該筆資料尚未公佈日期，則會被略過。
/// 最終結果會依照年份降序（新年度在前）排列。
pub async fn visit(stock_symbol: &str) -> Result<YahooDividend> {
    let url = format!("https://{}/quote/{}/dividend", HOST, stock_symbol);
    let text = http::get(&url, None).await?;
    parse_dividend_html(stock_symbol, &url, &text)
}

fn parse_dividend_html(stock_symbol: &str, url: &str, text: &str) -> Result<YahooDividend> {
    let document = Html::parse_document(text);
    parse_dividend_document(stock_symbol, url, &document)
}

fn parse_dividend_document(
    stock_symbol: &str,
    url: &str,
    document: &Html,
) -> Result<YahooDividend> {
    let rows = document.select(&LIST_SELECTOR).collect::<Vec<_>>();
    if rows.is_empty() {
        return Err(anyhow!(
            "No dividend data found for {}. Site structure might have changed at {}",
            stock_symbol,
            url
        ));
    }

    let mut dividend_by_year = HashMap::<i32, Vec<YahooDividendDetail>>::new();

    for element in rows {
        // 股利所屬期間 (例如 "2024Q4")，位於特定的 Class 容器中
        let period_raw = http::element::parse_value(
            &element,
            "div > div.Fxg\\(1\\).Fxs\\(1\\).Fxb\\(0\\%\\).Ta\\(end\\)",
        );
        if period_raw.is_none() {
            continue;
        }

        let (year_of_dividend, quarter) = parse_period(&period_raw)?;

        // 判定發放年度 (year)
        let mut year = 0;
        let (ex_div_date, ex_rights_date, pay_date1, pay_date2) = if !quarter.is_empty() {
            // 修正：季配或半年配，年度優先以發放日為準
            let pay_date1 = parse_dt(&element, 9, &mut year);
            let pay_date2 = parse_dt(&element, 10, &mut year);
            let ex_div_date = parse_dt(&element, 7, &mut year);
            let ex_rights_date = parse_dt(&element, 8, &mut year);

            (ex_div_date, ex_rights_date, pay_date1, pay_date2)
        } else {
            // 年度配息：維持原邏輯，優先以除息/除權日為準
            let ex_div_date = parse_dt(&element, 7, &mut year);
            let ex_rights_date = parse_dt(&element, 8, &mut year);
            let mut dummy = 0;
            let pay_date1 = parse_dt(&element, 9, &mut dummy);
            let pay_date2 = parse_dt(&element, 10, &mut dummy);

            (ex_div_date, ex_rights_date, pay_date1, pay_date2)
        };

        // 若無有效年份（代表日期皆尚未公佈），則略過此筆配息記錄
        if year == 0 {
            continue;
        }

        // 股利數值 (3=現金股利, 4=股票股利)
        let cash_dividend = parse_val(&element, 3);
        let stock_dividend = parse_val(&element, 4);

        dividend_by_year
            .entry(year)
            .or_default()
            .push(YahooDividendDetail {
                year,
                year_of_dividend,
                quarter,
                cash_dividend,
                stock_dividend,
                ex_dividend_date1: ex_div_date,
                ex_dividend_date2: ex_rights_date,
                payable_date1: pay_date1,
                payable_date2: pay_date2,
            });
    }

    let mut result = YahooDividend::new(stock_symbol.to_string());
    result.dividend = dividend_by_year.into_iter().collect();
    // 依年份降序排列，確保最新的股利資訊排在最前面
    result.dividend.sort_unstable_by(|(a, _), (b, _)| b.cmp(a));

    Ok(result)
}

/// 內部輔助：解析數值欄位並轉換為 `Decimal`。
fn parse_val(el: &scraper::ElementRef, child_idx: usize) -> Decimal {
    let selector = format!("div > div:nth-child({})", child_idx);
    let raw = http::element::parse_value(el, &selector);
    raw.as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "-")
        .and_then(|v| text::parse_decimal(v, None).ok())
        .unwrap_or(Decimal::ZERO)
}

/// 內部輔助：解析日期字串並提取年度。
///
/// 若傳入 `year_out` 為 0，會嘗試從日期 (YYYY/MM/DD) 提取年份填入。
/// 同時將日期格式由 `YYYY/MM/DD` 轉換為 `YYYY-MM-DD`。
fn parse_dt(el: &scraper::ElementRef, child_idx: usize, year_out: &mut i32) -> String {
    let selector = format!("div > div:nth-child({})", child_idx);
    let raw = http::element::parse_value(el, &selector);
    match raw {
        Some(s) if !s.is_empty() && s.contains('/') => {
            if *year_out == 0 {
                if let Some(y) = s.split('/').next().and_then(|y| y.parse::<i32>().ok()) {
                    *year_out = y;
                }
            }
            s.replace('/', "-")
        }
        _ => "-".to_string(),
    }
}

/// 解析股利期間字串（如 "2024Q4"），拆分為年度與季度。
fn parse_period(period: &Option<String>) -> Result<(i32, String)> {
    if let Some(p) = period {
        if let Some(caps) = REG_PERIOD.captures(p) {
            let year = caps.get(1).map_or(0, |m| m.as_str().parse().unwrap_or(0));
            let quarter = caps.get(2).map_or("", |m| m.as_str()).to_string();
            return Ok((year, quarter));
        }
    }
    Ok((0, "".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    fn dividend_row(
        period: &str,
        cash_dividend: &str,
        stock_dividend: &str,
        ex_dividend_date: &str,
        ex_rights_date: &str,
        payable_date1: &str,
        payable_date2: &str,
    ) -> String {
        format!(
            r#"
            <li>
                <div>
                    <div class="Fxg(1) Fxs(1) Fxb(0%) Ta(end)">{period}</div>
                    <div>unused</div>
                    <div>{cash_dividend}</div>
                    <div>{stock_dividend}</div>
                    <div>unused</div>
                    <div>unused</div>
                    <div>{ex_dividend_date}</div>
                    <div>{ex_rights_date}</div>
                    <div>{payable_date1}</div>
                    <div>{payable_date2}</div>
                </div>
            </li>
            "#
        )
    }

    fn wrap_rows(rows: &[String]) -> String {
        format!(
            r#"<div id="main-2-QuoteDividend-Proxy"><ul>{}</ul></div>"#,
            rows.join("")
        )
    }

    #[test]
    fn parse_dividend_html_groups_records_by_paid_year_and_sorts_desc() {
        let html = wrap_rows(&[
            dividend_row(
                "2024Q4",
                "1.5",
                "0.5",
                "2025/03/10",
                "-",
                "2025/04/11",
                "2025/05/01",
            ),
            dividend_row("2024Q3", "-", "1.2", "-", "2025/01/15", "-", "2025/02/20"),
            dividend_row("2023", "2.0", "-", "2024/07/01", "-", "2024/07/30", "-"),
            dividend_row("2022Q2", "1.0", "0.0", "-", "-", "-", "-"),
        ]);

        let dividend =
            parse_dividend_html("2330", "https://example.test/quote/2330/dividend", &html)
                .expect("expected parser to extract dividend rows");

        assert_eq!(dividend.stock_symbol, "2330");
        assert_eq!(dividend.dividend.len(), 2);
        assert_eq!(dividend.dividend[0].0, 2025);
        assert_eq!(dividend.dividend[1].0, 2024);

        let details_2025 = dividend
            .get_dividend_by_year(2025)
            .expect("expected grouped data for 2025");
        assert_eq!(details_2025.len(), 2);

        let q4 = &details_2025[0];
        assert_eq!(q4.year, 2025);
        assert_eq!(q4.year_of_dividend, 2024);
        assert_eq!(q4.quarter, "Q4");
        assert_eq!(q4.cash_dividend, dec!(1.5));
        assert_eq!(q4.stock_dividend, dec!(0.5));
        assert_eq!(q4.ex_dividend_date1, "2025-03-10");
        assert_eq!(q4.ex_dividend_date2, "-");
        assert_eq!(q4.payable_date1, "2025-04-11");
        assert_eq!(q4.payable_date2, "2025-05-01");

        let q3 = &details_2025[1];
        assert_eq!(q3.year, 2025);
        assert_eq!(q3.year_of_dividend, 2024);
        assert_eq!(q3.quarter, "Q3");
        assert_eq!(q3.cash_dividend, Decimal::ZERO);
        assert_eq!(q3.stock_dividend, dec!(1.2));
        assert_eq!(q3.ex_dividend_date1, "-");
        assert_eq!(q3.ex_dividend_date2, "2025-01-15");
        assert_eq!(q3.payable_date1, "-");
        assert_eq!(q3.payable_date2, "2025-02-20");

        let details_2024 = dividend
            .get_dividend_by_year(2024)
            .expect("expected grouped data for 2024");
        assert_eq!(details_2024.len(), 1);
        assert_eq!(details_2024[0].quarter, "");
        assert_eq!(details_2024[0].year_of_dividend, 2023);
        assert_eq!(details_2024[0].cash_dividend, dec!(2.0));
        assert_eq!(details_2024[0].stock_dividend, Decimal::ZERO);
        assert_eq!(details_2024[0].ex_dividend_date1, "2024-07-01");
        assert_eq!(details_2024[0].payable_date1, "2024-07-30");

        assert!(dividend.get_dividend_by_year(2022).is_none());
    }

    #[test]
    fn parse_dividend_html_returns_error_when_dividend_list_is_missing() {
        let err = parse_dividend_html(
            "2330",
            "https://example.test/quote/2330/dividend",
            r#"<div id="main-2-QuoteDividend-Proxy"><ul></ul></div>"#,
        )
        .expect_err("expected parser to reject empty dividend list");

        let message = err.to_string();
        assert!(message.contains("2330"));
        assert!(message.contains("https://example.test/quote/2330/dividend"));
    }

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 visit".to_string());

        match visit("5306").await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("{:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because {:?}", why));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
    }
}
