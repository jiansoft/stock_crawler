//! # MoneyDJ 股利政策表（除權除息日程）採集器
//!
//! 資料來源為 <https://www.moneydj.com/Z/ZE/ZEY/ZEY.djhtm>。
//!
//! ## 這支頁面補的是什麼
//!
//! 交易所的除權除息預告表與 MOPS 的股利分派情形都**沒有現金股利發放日**，
//! 而 `dividend.payable_date1` 需要它。這張表是目前唯一一次就能拿到全市場
//! 「除權息日 → 現金股利發放日」對應的來源。
//!
//! ## 已知限制與陷阱
//!
//! - 頁面是 **Big5** 編碼，必須用 [`crate::core::util::http::get_use_big5`] 抓取。
//! - 股票代號藏在 `GenLink2stk('AS2442','新美齊')` 這段 JavaScript 裡，
//!   而且前綴不固定：一般股票是 `AS`，ETF／REIT 是 `AP`，因此只能略過前兩碼。
//! - 只涵蓋**未來一段時間**的預告（實測約三個半月），無法用來回補歷史資料。
//! - 「現金股利發放日」欄位在**純除權**的資料列上，填的是該公司**另一次除息**的發放日
//!   （備註欄會寫成「07/29除息3元」）。若照單全收會把不相干的日期綁到除權事件上，
//!   因此本模組只在資料列含「息」時才採用這個欄位。
//! - 發放日只有 `MM/DD`，年份要用除權息日推算；跨年（例如 12 月除息、隔年 1 月發放）
//!   時月日會比除權息日小，此時年份加一。

use anyhow::Result;
use chrono::{Datelike, NaiveDate};
use once_cell::sync::Lazy;
use regex::Regex;
use rust_decimal::Decimal;
use scraper::{Html, Selector};

use crate::{core::util, infra::crawler::moneydj};

/// 從 `GenLink2stk('AS2442','新美齊')` 取出代號與名稱。
///
/// 前兩碼是 MoneyDJ 的市場前綴（`AS` 一般股票、`AP` ETF/REIT 等），一律略過。
static SYMBOL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"GenLink2stk\('[A-Za-z]{2}([0-9A-Za-z]+)','([^']*)'\)").unwrap());

/// 從「股票：0.2」取出股票股利。
static STOCK_DIVIDEND_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"股票：([0-9.]+)").unwrap());

/// 從「現金：2.03186184」取出現金股利。
static CASH_DIVIDEND_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"現金：([0-9.]+)").unwrap());

/// 表格列選擇器。
static ROW_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("tr").unwrap());

/// 資料格選擇器。
static CELL_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("td").unwrap());

/// 單一檔股票的一次除權息日程。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DividendSchedule {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 除權除息交易日。
    pub ex_date: NaiveDate,
    /// 是否為除息。
    pub is_cash: bool,
    /// 是否為除權。
    pub is_stock: bool,
    /// 現金股利（元/股）。
    pub cash_dividend: Option<Decimal>,
    /// 股票股利（元/股）。
    pub stock_dividend: Option<Decimal>,
    /// 現金股利發放日；純除權的資料列一律為 `None`（見模組說明）。
    pub cash_payable_date: Option<NaiveDate>,
}

/// 「股利」欄位在表格中的索引。
const COLUMN_KIND: usize = 1;
/// 除權息日期欄位索引。
const COLUMN_EX_DATE: usize = 2;
/// 股利金額欄位索引。
const COLUMN_DIVIDEND: usize = 3;
/// 現金股利發放日欄位索引。
const COLUMN_CASH_PAYABLE_DATE: usize = 8;

/// 取得 MoneyDJ 股利政策表。
///
/// # 錯誤
///
/// 當 HTTP 請求或 Big5 解碼失敗時回傳錯誤；個別資料列解析失敗只會被略過，
/// 不會讓整批資料失敗。
pub async fn visit() -> Result<Vec<DividendSchedule>> {
    let url = format!(
        "https://www.{host}/Z/ZE/ZEY/ZEY.djhtm",
        host = moneydj::WEB_HOST
    );
    let text = util::http::get_use_big5(&url).await?;

    Ok(parse_schedules(&text))
}

/// 解析股利政策表的 HTML。
///
/// 這是純函式，可用 `testdata/dividend_schedule_zey.html` fixture 直接驗證。
/// 沒有股票代號的資料列（表頭、版面用的空列）會被略過。
fn parse_schedules(html: &str) -> Vec<DividendSchedule> {
    let document = Html::parse_document(html);
    let mut result = Vec::with_capacity(256);

    for row in document.select(&ROW_SELECTOR) {
        // 代號寫在 <script> 內，取整列 HTML 比逐格找可靠。
        // inner_html() 產生的是暫存字串，必須先綁定，captures 才能安全借用它。
        let row_html = row.inner_html();
        let Some(captures) = SYMBOL_RE.captures(&row_html) else {
            continue;
        };
        let stock_symbol = captures[1].to_string();
        let name = captures[2].to_string();

        let cells: Vec<String> = row
            .select(&CELL_SELECTOR)
            .map(|cell| cell.text().collect::<String>().trim().to_string())
            .collect();
        if cells.len() <= COLUMN_CASH_PAYABLE_DATE {
            continue;
        }

        let Some(ex_date) = parse_date(&cells[COLUMN_EX_DATE]) else {
            continue;
        };

        let kind = &cells[COLUMN_KIND];
        let (is_cash, is_stock) = crate::infra::crawler::share::parse_ex_dividend_kind(kind);
        let dividend_text = &cells[COLUMN_DIVIDEND];

        // 純除權列的「現金股利發放日」屬於另一次除息事件，必須忽略。
        let cash_payable_date = if is_cash {
            parse_payable_date(&cells[COLUMN_CASH_PAYABLE_DATE], ex_date)
        } else {
            None
        };

        result.push(DividendSchedule {
            stock_symbol,
            name,
            ex_date,
            is_cash,
            is_stock,
            cash_dividend: capture_decimal(&CASH_DIVIDEND_RE, dividend_text),
            stock_dividend: capture_decimal(&STOCK_DIVIDEND_RE, dividend_text),
            cash_payable_date,
        });
    }

    result
}

/// 解析 `2026/09/08` 形式的西元日期。
fn parse_date(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value.trim(), "%Y/%m/%d").ok()
}

/// 解析只有 `MM/DD` 的發放日，並以除權息日推算年份。
///
/// 發放日一定在除權息日之後；若月日比除權息日小，代表跨到隔年。
fn parse_payable_date(value: &str, ex_date: NaiveDate) -> Option<NaiveDate> {
    let trimmed = value.trim();
    let mut parts = trimmed.split('/');
    let month = parts.next()?.trim().parse::<u32>().ok()?;
    let day = parts.next()?.trim().parse::<u32>().ok()?;
    if parts.next().is_some() {
        return None;
    }

    let year = if (month, day) < (ex_date.month(), ex_date.day()) {
        ex_date.year() + 1
    } else {
        ex_date.year()
    };

    NaiveDate::from_ymd_opt(year, month, day)
}

/// 用指定的正則從股利欄位取出數值。
fn capture_decimal(re: &Regex, text: &str) -> Option<Decimal> {
    let captures = re.captures(text)?;
    util::text::parse_decimal(&captures[1], None).ok()
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    /// 以真實頁面片段 fixture 驗證解析結果。
    ///
    /// 涵蓋：除權息（同時有股票與現金）、純除息、ETF 的 `AP` 前綴代號、
    /// 純除權列必須忽略不相干的現金股利發放日。
    #[test]
    fn test_parse_schedules_with_fixture() {
        // include_str! 的路徑相對於本檔案（moneydj/dividend_schedule.rs）→ moneydj/testdata/。
        const FIXTURE: &str = include_str!("testdata/dividend_schedule_zey.html");
        let result = parse_schedules(FIXTURE);

        assert_eq!(result.len(), 4);

        // 除權息：股票與現金股利都要取到，發放日 10/08 與除權息日同年。
        let both = &result[0];
        assert_eq!(both.stock_symbol, "2442");
        assert_eq!(both.name, "新美齊");
        assert_eq!(both.ex_date, NaiveDate::from_ymd_opt(2026, 9, 8).unwrap());
        assert!(both.is_cash);
        assert!(both.is_stock);
        assert_eq!(both.cash_dividend, Some(dec!(2.03186184)));
        assert_eq!(both.stock_dividend, Some(dec!(0.711151634)));
        assert_eq!(both.cash_payable_date, NaiveDate::from_ymd_opt(2026, 10, 8));

        // ETF 的連結前綴是 AP，代號一樣要正確取出。
        let etf = result
            .iter()
            .find(|item| item.stock_symbol == "00400A")
            .expect("00400A should exist");
        assert!(etf.is_cash);
        assert!(!etf.is_stock);
        assert_eq!(etf.cash_dividend, Some(dec!(0.12)));
        assert_eq!(etf.stock_dividend, None);

        // 純除權列：頁面上填的發放日屬於另一次除息，必須忽略。
        let stock_only = result
            .iter()
            .find(|item| !item.is_cash && item.is_stock)
            .expect("stock-only row should exist");
        assert_eq!(stock_only.stock_symbol, "4549");
        assert_eq!(stock_only.stock_dividend, Some(dec!(0.2)));
        assert!(stock_only.cash_payable_date.is_none());
    }

    /// 跨年發放：12 月除息、隔年 1 月發放，年份要加一。
    #[test]
    fn test_parse_payable_date_rolls_over_year() {
        let ex_date = NaiveDate::from_ymd_opt(2026, 12, 20).unwrap();

        assert_eq!(
            parse_payable_date("01/15", ex_date),
            NaiveDate::from_ymd_opt(2027, 1, 15)
        );
        assert_eq!(
            parse_payable_date("12/25", ex_date),
            NaiveDate::from_ymd_opt(2026, 12, 25)
        );
        assert_eq!(parse_payable_date("", ex_date), None);
        assert_eq!(parse_payable_date("2026/12/25", ex_date), None);
    }

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenvy::dotenv().ok();

        match visit().await {
            Err(why) => println!("取得 MoneyDJ 股利政策表失敗: {:?}", why),
            Ok(result) => {
                println!("股利政策表 {} 筆", result.len());
                for item in result.iter().take(5) {
                    println!("{:?}", item);
                }
            }
        }
    }
}
