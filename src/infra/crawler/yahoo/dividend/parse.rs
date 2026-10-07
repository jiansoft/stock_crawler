//! Yahoo 股利頁的 HTML 解析：擷取每一列配息，依發放年度分組後交給 [`super::periods`] 整理期別。

use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow};
use once_cell::sync::Lazy;
use regex::Regex;
use rust_decimal::Decimal;
use scraper::{Html, Selector};

use super::periods::{merge_annual_event_into_last_period, resolve_repeated_periods};
use super::{YahooDividend, YahooDividendDetail};
use crate::core::util::{http, text};

/// 用於解析股利所屬期間（如 2024Q4）的正則表達式
static REG_PERIOD: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(\d{4})(Q\d|H\d|M\d{1,2})?").expect("Failed to compile dividend period regex")
});

/// 股利列表明細行的選擇器
static LIST_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse("#main-2-QuoteDividend-Proxy ul > li")
        .expect("Failed to parse dividend list selector")
});

/// 解析下載的股利 HTML，供採集與離線測試共用，並傳回依發放年度分組的明細。
pub(super) fn parse_dividend_html(
    stock_symbol: &str,
    url: &str,
    text: &str,
) -> Result<YahooDividend> {
    let document = Html::parse_document(text);
    parse_dividend_document(stock_symbol, url, &document)
}

/// 尚未決定發放年度的解析結果。
///
/// 發放年度必須等整張表看完才能決定：Yahoo 會把「已宣告但日期未定」的股利
/// 另外列在表格最上方，該列只能用所屬年度推估，但同一次配發稍後還會以確定日期再出現一次。
struct PendingDividendRow {
    /// 由實際日期解析出的發放年度；`None` 代表 Yahoo 尚未公布任何日期。
    resolved_year: Option<i32>,
    year_of_dividend: i32,
    quarter: String,
    cash_dividend: Decimal,
    stock_dividend: Decimal,
    ex_dividend_date1: String,
    ex_dividend_date2: String,
    payable_date1: String,
    payable_date2: String,
}

/// 從股利列表擷取實際配息事件，略過年度合計，避免合計被當作另一筆配息。
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

    let mut pending: Vec<PendingDividendRow> = Vec::with_capacity(rows.len());

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
        // Yahoo 合計列也有期間容器，但內容是空字串；沒有有效所屬年度就不是配息明細。
        if year_of_dividend == 0 {
            continue;
        }

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

        // 股利數值 (3=現金股利, 4=股票股利)
        let cash_dividend = parse_val(&element, 3);
        let stock_dividend = parse_val(&element, 4);

        pending.push(PendingDividendRow {
            resolved_year: (year != 0).then_some(year),
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

    // 同一個所屬期間若已有「日期確定」的列，該期間的推估列就是同一次配發的舊快照
    // （Yahoo 把尚未公布日期的股利另列在表格最上方）。兩列的主鍵相同，一起入庫只會互相覆蓋，
    // 例如 2753 的 2025H2 會在現金 7.5 與配股 0.5 之間來回。日期確定的那一列才是完整資料。
    let resolved_periods: HashSet<(i32, &str)> = pending
        .iter()
        .filter(|row| row.resolved_year.is_some())
        .map(|row| (row.year_of_dividend, row.quarter.as_str()))
        .collect();

    let mut dividend_by_year = HashMap::<i32, Vec<YahooDividendDetail>>::new();
    for row in &pending {
        let year = match row.resolved_year {
            Some(year) => year,
            None => {
                if resolved_periods.contains(&(row.year_of_dividend, row.quarter.as_str())) {
                    continue;
                }
                // 日期全部尚未公布時，Yahoo 仍可能先揭露擬定股利；先以所屬年度加一推估發放年度，
                // 讓擬定股利也能入庫，待日期公布後再由回補流程換成確定資料。
                estimate_paid_year(row.year_of_dividend)
            }
        };

        dividend_by_year
            .entry(year)
            .or_default()
            .push(YahooDividendDetail {
                year,
                year_of_dividend: row.year_of_dividend,
                quarter: row.quarter.clone(),
                cash_dividend: row.cash_dividend,
                stock_dividend: row.stock_dividend,
                ex_dividend_date1: row.ex_dividend_date1.clone(),
                ex_dividend_date2: row.ex_dividend_date2.clone(),
                payable_date1: row.payable_date1.clone(),
                payable_date2: row.payable_date2.clone(),
            });
    }

    let is_etf = stock_symbol.starts_with("00");
    for details in dividend_by_year.values_mut() {
        resolve_repeated_periods(details, is_etf);
        merge_annual_event_into_last_period(details);

        // 同一發放年同時有全年與季／半年配時，全年事件使用資料表既有的 A 代碼。
        // 空季度留給年度合計，避免例如 2072 的 2025 年配被 2026H1 合計覆蓋。
        if details.iter().any(|detail| !detail.quarter.is_empty()) {
            for detail in details {
                if detail.quarter.is_empty() {
                    detail.quarter = "A".to_string();
                }
            }
        }
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
/// 同時將日期格式由 `YYYY/MM/DD` 轉換為 `YYYY-MM-DD`。若 Yahoo 明確標示 `尚未公布`，
/// 會保留原值，讓後續資料庫查詢能每日重新採集該筆股利的除息日或發放日。
fn parse_dt(el: &scraper::ElementRef, child_idx: usize, year_out: &mut i32) -> String {
    let selector = format!("div > div:nth-child({})", child_idx);
    let raw = http::element::parse_value(el, &selector);
    match raw {
        Some(s) if s.trim() == "尚未公布" => "尚未公布".to_string(),
        Some(s) if !s.is_empty() && s.contains('/') => {
            if *year_out == 0
                && let Some(y) = s.split('/').next().and_then(|y| y.parse::<i32>().ok())
            {
                *year_out = y;
            }
            s.replace('/', "-")
        }
        _ => "-".to_string(),
    }
}

/// 解析股利期間字串（如 "2024Q4"），拆分為年度與季度。
fn parse_period(period: &Option<String>) -> Result<(i32, String)> {
    if let Some(p) = period
        && let Some(caps) = REG_PERIOD.captures(p)
    {
        let year = caps.get(1).map_or(0, |m| m.as_str().parse().unwrap_or(0));
        let quarter = caps.get(2).map_or("", |m| m.as_str());
        // 月配統一補成兩位數（2026M9 → M09），期別字串的字典序才等於時間序。
        let quarter = match quarter
            .strip_prefix('M')
            .and_then(|month| month.parse::<u32>().ok())
        {
            Some(month) => format!("M{month:02}"),
            None => quarter.to_string(),
        };
        return Ok((year, quarter));
    }
    Ok((0, "".to_string()))
}

/// 從股利所屬年度推估發放年度。
///
/// Yahoo 對尚未公布除權息日的擬定股利，發放期間可能顯示 `-`，但仍會提供股利所屬年度與股利金額。
/// 在無法從日期判斷實際發放年度時，先以 `year_of_dividend + 1` 建立可入庫的暫定發放年度，
/// 後續日期公布後再由回補流程更新成實際日期資料。
fn estimate_paid_year(year_of_dividend: i32) -> i32 {
    year_of_dividend + 1
}
