//! 盈餘分配率：每筆股利除以「它涵蓋的盈餘期間」的每股盈餘。
//!
//! 分母不能固定取「單季」或「前一年全年」：
//!
//! - 季配息公司（例如 4735 豪展）每季各配一次，每筆只涵蓋單季。同一發放年度的年度合計列
//!   混了前一年 Q4 與今年 Q1、Q2 的股利，分母必須是這幾季 EPS 的合計；若拿前一年全年 EPS
//!   當分母，2026 年會算出 121.95%（實際 58.71%）。
//! - 章程改為可按季配息、實際一年只配一次的公司（例如 1102 亞泥），來源把那筆股利標成 Q4，
//!   但它分配的是全年累積的盈餘；只用 Q4 單季 EPS 會算出 395%（實際 76.67%）。
//!
//! 因此一筆股利涵蓋的期間是「同一所屬年度上一次配息之後的下一季」到「這一期」為止；
//! 年度合計列則是組成它的每一筆配息涵蓋期間的聯集。分母 EPS 與涵蓋期間一併寫回資料庫，
//! 畫面顯示的 EPS 才會與分配率出自同一組數字。

use std::collections::{HashMap, HashSet};

use rust_decimal::Decimal;

/// 計算盈餘分配率所需的股利列（含目前已寫入的結果，用來判斷是否需要更新）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayoutDividend {
    /// 股利列序號。
    pub serial: i64,
    /// 證券代號。
    pub security_code: String,
    /// 發放年度。
    pub year: i32,
    /// 股利所屬年度。
    pub year_of_dividend: i32,
    /// 期別：Q1~Q4、H1/H2、A（全年）；空字串為全年配息，或同一發放年度有明細時的年度合計。
    pub quarter: String,
    /// 現金股利。
    pub cash_dividend: Decimal,
    /// 股票股利。
    pub stock_dividend: Decimal,
    /// 股利合計。
    pub sum: Decimal,
    /// 目前的盈餘分配率_配息(%)。
    pub payout_ratio_cash: Decimal,
    /// 目前的盈餘分配率_配股(%)。
    pub payout_ratio_stock: Decimal,
    /// 目前的盈餘分配率(%)。
    pub payout_ratio: Decimal,
    /// 目前寫入的分母 EPS；尚未計算過時為 `None`。
    pub payout_eps: Option<Decimal>,
    /// 目前寫入的涵蓋期間；尚未計算過時為 `None`。
    pub payout_period: Option<String>,
}

/// 財報每股盈餘。`quarter` 為 Q1~Q4，空字串代表年報的全年 EPS。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodEarnings {
    /// 證券代號。
    pub security_code: String,
    /// 財報年度。
    pub year: i32,
    /// 季別（Q1~Q4），空字串為全年。
    pub quarter: String,
    /// 每股盈餘。
    pub earnings_per_share: Decimal,
}

/// 計算完成、需要寫回的盈餘分配率。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayoutRatios {
    /// 股利列序號。
    pub serial: i64,
    /// 盈餘分配率_配息(%)。
    pub payout_ratio_cash: Decimal,
    /// 盈餘分配率_配股(%)。
    pub payout_ratio_stock: Decimal,
    /// 盈餘分配率(%)。
    pub payout_ratio: Decimal,
    /// 分母：涵蓋期間的每股盈餘。
    pub payout_eps: Decimal,
    /// 涵蓋期間，例如 `2025`、`2026Q2`、`2026Q1~Q2`、`2025Q4~2026Q2`。
    pub payout_period: String,
}

/// 年度中的一季（`quarter` 為 1~4），可依時間先後比較。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct QuarterPoint {
    year: i32,
    quarter: u8,
}

/// 一筆股利涵蓋的盈餘期間與該期間的 EPS 合計。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Coverage {
    start: QuarterPoint,
    end: QuarterPoint,
    eps: Decimal,
}

/// `(年度, 季)` → EPS；季為 0 代表年報的全年 EPS。
type EarningsBook = HashMap<(i32, u8), Decimal>;

/// 依涵蓋期間重算每一筆股利的盈餘分配率，只回傳與資料庫現值不同、需要寫回的列。
///
/// 涵蓋期間的 EPS 算不出來（財報還沒公布、季別不完整）的列不回傳，保留原值等下次排程；
/// 算得出來但不為正（虧損仍配息）時分配率寫 0——負值或無限大的比率只會污染下游統計，
/// 分母 EPS 則照實寫回，畫面才看得出是虧損期間配的股利。
pub fn calculate_payout_ratios(
    dividends: &[PayoutDividend],
    earnings: &[PeriodEarnings],
) -> Vec<PayoutRatios> {
    let mut books: HashMap<&str, EarningsBook> = HashMap::new();
    for row in earnings {
        let quarter = match row.quarter.as_str() {
            "" => 0,
            "Q1" => 1,
            "Q2" => 2,
            "Q3" => 3,
            "Q4" => 4,
            _ => continue,
        };
        books
            .entry(row.security_code.as_str())
            .or_default()
            .insert((row.year, quarter), row.earnings_per_share);
    }

    let mut by_stock: HashMap<&str, Vec<&PayoutDividend>> = HashMap::new();
    for row in dividends {
        by_stock
            .entry(row.security_code.as_str())
            .or_default()
            .push(row);
    }

    let mut result: Vec<PayoutRatios> = by_stock
        .into_iter()
        .filter_map(|(code, rows)| books.get(code).map(|book| (rows, book)))
        .flat_map(|(rows, book)| calculate_for_stock(&rows, book))
        .collect();
    result.sort_by_key(|ratios| ratios.serial);
    result
}

/// 計算單一股票的所有股利列。
fn calculate_for_stock(rows: &[&PayoutDividend], book: &EarningsBook) -> Vec<PayoutRatios> {
    // 同一發放年度有明細（季別非空）時，空季別那一列是年度合計，不是一次配息。
    let years_with_installments: HashSet<i32> = rows
        .iter()
        .filter(|row| !row.quarter.is_empty())
        .map(|row| row.year)
        .collect();
    let is_annual_total = |row: &PayoutDividend| {
        row.quarter.is_empty() && years_with_installments.contains(&row.year)
    };

    // 配息事件：排除年度合計；沒有配發（sum = 0）的列不算一次配息，也不影響其他列的涵蓋期間。
    let events: Vec<(&PayoutDividend, u8)> = rows
        .iter()
        .copied()
        .filter(|row| !is_annual_total(row) && row.year_of_dividend > 0 && row.sum > Decimal::ZERO)
        .filter_map(|row| period_end_quarter(&row.quarter).map(|end| (row, end)))
        .collect();

    let coverages: HashMap<i64, Coverage> = events
        .iter()
        .filter_map(|(row, end)| {
            event_coverage(row, *end, &events, book).map(|coverage| (row.serial, coverage))
        })
        .collect();

    let mut result: Vec<PayoutRatios> = events
        .iter()
        .filter_map(|(row, _)| {
            coverages
                .get(&row.serial)
                .and_then(|coverage| changed_ratios(row, coverage))
        })
        .collect();

    for total in rows
        .iter()
        .copied()
        .filter(|row| is_annual_total(row) && row.sum > Decimal::ZERO)
    {
        let installments: Vec<&Coverage> = events
            .iter()
            .filter(|(row, _)| row.year == total.year)
            .map(|(row, _)| coverages.get(&row.serial))
            .collect::<Option<Vec<_>>>()
            .unwrap_or_default();
        let Some(coverage) = merge_coverages(&installments) else {
            continue;
        };
        if let Some(ratios) = changed_ratios(total, &coverage) {
            result.push(ratios);
        }
    }

    result
}

/// 期別結束於第幾季；無法辨識的期別回傳 `None`（不計算）。
fn period_end_quarter(quarter: &str) -> Option<u8> {
    match quarter {
        "Q1" => Some(1),
        "Q2" | "H1" => Some(2),
        "Q3" => Some(3),
        "Q4" | "H2" | "A" | "" => Some(4),
        _ => None,
    }
}

/// 一次配息涵蓋的期間：同一所屬年度上一次配息之後的下一季，到這一期結束為止。
fn event_coverage(
    row: &PayoutDividend,
    end: u8,
    events: &[(&PayoutDividend, u8)],
    book: &EarningsBook,
) -> Option<Coverage> {
    let year = row.year_of_dividend;
    let start = events
        .iter()
        .filter(|(other, other_end)| {
            other.serial != row.serial && other.year_of_dividend == year && *other_end < end
        })
        .map(|(_, other_end)| *other_end)
        .max()
        .map_or(1, |previous_end| previous_end + 1);

    // 涵蓋全年時優先採年報 EPS：四季相加會因每季各用自己的加權平均股數而系統性偏高。
    let eps = if start == 1 && end == 4 {
        book.get(&(year, 0))
            .copied()
            .or_else(|| sum_quarters(book, year, start, end))
    } else {
        sum_quarters(book, year, start, end)
    }?;

    Some(Coverage {
        start: QuarterPoint {
            year,
            quarter: start,
        },
        end: QuarterPoint { year, quarter: end },
        eps,
    })
}

/// 同一年度 `start..=end` 各季 EPS 的合計；缺任何一季就回傳 `None`。
fn sum_quarters(book: &EarningsBook, year: i32, start: u8, end: u8) -> Option<Decimal> {
    (start..=end)
        .map(|quarter| book.get(&(year, quarter)))
        .sum()
}

/// 年度合計列的涵蓋期間：各筆配息涵蓋期間的聯集，EPS 為各筆 EPS 相加。
fn merge_coverages(installments: &[&Coverage]) -> Option<Coverage> {
    let start = installments.iter().map(|coverage| coverage.start).min()?;
    let end = installments.iter().map(|coverage| coverage.end).max()?;
    let eps = installments.iter().map(|coverage| coverage.eps).sum();
    Some(Coverage { start, end, eps })
}

/// 涵蓋期間的顯示文字：全年 `2025`、單季 `2026Q2`、同年多季 `2026Q1~Q2`、跨年 `2025Q4~2026Q2`。
fn period_label(coverage: &Coverage) -> String {
    let Coverage { start, end, .. } = coverage;
    if start.year == end.year {
        return match (start.quarter, end.quarter) {
            (1, 4) => start.year.to_string(),
            (first, last) if first == last => format!("{}Q{}", start.year, first),
            (first, last) => format!("{}Q{}~Q{}", start.year, first, last),
        };
    }
    format!(
        "{}Q{}~{}Q{}",
        start.year, start.quarter, end.year, end.quarter
    )
}

/// 依涵蓋期間算出分配率；與資料庫現值完全相同時回傳 `None`，不必寫回。
fn changed_ratios(row: &PayoutDividend, coverage: &Coverage) -> Option<PayoutRatios> {
    let eps = coverage.eps.round_dp(4);
    let ratio = |dividend: Decimal| {
        if eps > Decimal::ZERO {
            (dividend / eps * Decimal::ONE_HUNDRED).round_dp(4)
        } else {
            Decimal::ZERO
        }
    };

    let ratios = PayoutRatios {
        serial: row.serial,
        payout_ratio_cash: ratio(row.cash_dividend),
        payout_ratio_stock: ratio(row.stock_dividend),
        payout_ratio: ratio(row.sum),
        payout_eps: eps,
        payout_period: period_label(coverage),
    };

    let unchanged = row.payout_ratio_cash == ratios.payout_ratio_cash
        && row.payout_ratio_stock == ratios.payout_ratio_stock
        && row.payout_ratio == ratios.payout_ratio
        && row.payout_eps == Some(ratios.payout_eps)
        && row.payout_period.as_deref() == Some(ratios.payout_period.as_str());
    (!unchanged).then_some(ratios)
}

#[cfg(test)]
mod tests;
