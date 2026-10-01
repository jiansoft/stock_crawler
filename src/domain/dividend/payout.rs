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
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn dividend(
        serial: i64,
        year: i32,
        year_of_dividend: i32,
        quarter: &str,
        cash: Decimal,
    ) -> PayoutDividend {
        PayoutDividend {
            serial,
            security_code: "4735".to_string(),
            year,
            year_of_dividend,
            quarter: quarter.to_string(),
            cash_dividend: cash,
            stock_dividend: Decimal::ZERO,
            sum: cash,
            payout_ratio_cash: Decimal::ZERO,
            payout_ratio_stock: Decimal::ZERO,
            payout_ratio: Decimal::ZERO,
            payout_eps: None,
            payout_period: None,
        }
    }

    fn earnings(year: i32, quarter: &str, eps: Decimal) -> PeriodEarnings {
        PeriodEarnings {
            security_code: "4735".to_string(),
            year,
            quarter: quarter.to_string(),
            earnings_per_share: eps,
        }
    }

    /// 豪展 2025～2026 年報與季報 EPS（正式庫與 Yahoo 核對過）。
    fn hao_zhan_earnings() -> Vec<PeriodEarnings> {
        vec![
            earnings(2024, "Q1", dec!(-0.15)),
            earnings(2024, "Q2", dec!(0.62)),
            earnings(2024, "Q3", dec!(0.56)),
            earnings(2024, "Q4", dec!(0.58)),
            earnings(2024, "", dec!(1.62)),
            earnings(2025, "Q1", dec!(0.29)),
            earnings(2025, "Q2", dec!(0.51)),
            earnings(2025, "Q3", dec!(0.9)),
            earnings(2025, "Q4", dec!(0.74)),
            earnings(2025, "", dec!(2.46)),
            earnings(2026, "Q1", dec!(1.19)),
            earnings(2026, "Q2", dec!(3.18)),
        ]
    }

    fn find(result: &[PayoutRatios], serial: i64) -> &PayoutRatios {
        result
            .iter()
            .find(|ratios| ratios.serial == serial)
            .unwrap_or_else(|| panic!("serial {serial} 應該要有結果：{result:?}"))
    }

    /// 豪展 2026 年發放 2025Q4 0.5、2026Q1 1.0、2026Q2 1.5，合計 3.0。
    /// 分母是這三季 EPS 0.74 + 1.19 + 3.18 = 5.11，不是 2025 年全年 EPS 2.46（那會算出 121.95%）。
    #[test]
    fn annual_total_of_quarterly_payer_uses_covered_quarters() {
        let dividends = vec![
            dividend(1, 2025, 2025, "Q3", dec!(0.7)),
            dividend(2, 2026, 2025, "Q4", dec!(0.5)),
            dividend(3, 2026, 2026, "Q1", dec!(1)),
            dividend(4, 2026, 2026, "Q2", dec!(1.5)),
            dividend(5, 2026, 2025, "", dec!(3)),
        ];

        let result = calculate_payout_ratios(&dividends, &hao_zhan_earnings());

        let total = find(&result, 5);
        assert_eq!(total.payout_eps, dec!(5.11));
        assert_eq!(total.payout_period, "2025Q4~2026Q2");
        assert_eq!(total.payout_ratio, dec!(58.7084));

        // 每一筆季配只涵蓋單季：Q4 前面已有 Q3 配息，不會把整年算進來。
        let q4 = find(&result, 2);
        assert_eq!(q4.payout_eps, dec!(0.74));
        assert_eq!(q4.payout_period, "2025Q4");
        assert_eq!(q4.payout_ratio, dec!(67.5676));
    }

    /// 1102 亞泥：來源把一年一次的股利標成 Q4，但同一所屬年度沒有更早的配息，
    /// 這筆涵蓋的是全年盈餘，分母取年報 EPS，而不是 Q4 單季 EPS（那會算出 395%）。
    #[test]
    fn lone_q4_dividend_covers_whole_year() {
        let dividends = vec![
            dividend(1, 2026, 2025, "Q4", dec!(2.3)),
            dividend(2, 2026, 2025, "", dec!(2.3)),
        ];
        let earnings = vec![
            earnings(2025, "Q1", dec!(0.6)),
            earnings(2025, "Q2", dec!(0.8)),
            earnings(2025, "Q3", dec!(1)),
            earnings(2025, "Q4", dec!(0.58)),
            earnings(2025, "", dec!(3)),
        ];

        let result = calculate_payout_ratios(&dividends, &earnings);

        for serial in [1, 2] {
            let ratios = find(&result, serial);
            assert_eq!(ratios.payout_eps, dec!(3));
            assert_eq!(ratios.payout_period, "2025");
            assert_eq!(ratios.payout_ratio, dec!(76.6667));
        }
    }

    /// 同一所屬年度前面沒有配息時，Q3 那筆涵蓋 Q1～Q3（豪展 2024 年 Q1 虧損、Q1/Q2 都沒配）。
    #[test]
    fn first_dividend_of_year_covers_earlier_quarters() {
        let dividends = vec![dividend(1, 2024, 2024, "Q3", dec!(0.45))];

        let result = calculate_payout_ratios(&dividends, &hao_zhan_earnings());

        let q3 = find(&result, 1);
        assert_eq!(q3.payout_eps, dec!(1.03));
        assert_eq!(q3.payout_period, "2024Q1~Q3");
    }

    /// 一般年配息（只有一列、季別空白）：分母是股利所屬年度的年報 EPS。
    #[test]
    fn plain_annual_dividend_uses_annual_eps() {
        let dividends = vec![dividend(1, 2025, 2024, "", dec!(1.25))];

        let result = calculate_payout_ratios(&dividends, &hao_zhan_earnings());

        let annual = find(&result, 1);
        assert_eq!(annual.payout_eps, dec!(1.62));
        assert_eq!(annual.payout_period, "2024");
        assert_eq!(annual.payout_ratio, dec!(77.1605));
    }

    /// 半年配：H1 涵蓋 Q1～Q2，H2 涵蓋 Q3～Q4。
    #[test]
    fn half_year_dividends_cover_two_quarters() {
        let dividends = vec![
            dividend(1, 2025, 2025, "H1", dec!(0.4)),
            dividend(2, 2026, 2025, "H2", dec!(0.8)),
        ];

        let result = calculate_payout_ratios(&dividends, &hao_zhan_earnings());

        assert_eq!(find(&result, 1).payout_eps, dec!(0.8));
        assert_eq!(find(&result, 1).payout_period, "2025Q1~Q2");
        assert_eq!(find(&result, 2).payout_eps, dec!(1.64));
        assert_eq!(find(&result, 2).payout_period, "2025Q3~Q4");
    }

    /// 涵蓋期間缺任何一季的財報就不計算，保留原值等下次排程；年度合計也跟著不算。
    #[test]
    fn missing_quarter_leaves_row_untouched() {
        let dividends = vec![
            dividend(1, 2026, 2026, "Q3", dec!(1)),
            dividend(2, 2026, 2026, "Q2", dec!(1.5)),
            dividend(3, 2026, 2025, "", dec!(2.5)),
        ];

        let result = calculate_payout_ratios(&dividends, &hao_zhan_earnings());

        assert!(result.iter().all(|ratios| ratios.serial == 2), "{result:?}");
    }

    /// 虧損期間仍配息：分母照實寫回，分配率寫 0（負值會污染下游統計）。
    #[test]
    fn loss_period_writes_zero_ratio_with_actual_eps() {
        let dividends = vec![dividend(1, 2024, 2024, "Q1", dec!(0.3))];

        let result = calculate_payout_ratios(&dividends, &hao_zhan_earnings());

        let q1 = find(&result, 1);
        assert_eq!(q1.payout_eps, dec!(-0.15));
        assert_eq!(q1.payout_ratio, Decimal::ZERO);
    }

    /// 已經寫過相同結果的列不重複更新；沒有財報的股票（ETF）整檔略過。
    #[test]
    fn unchanged_rows_and_stocks_without_earnings_are_skipped() {
        let mut computed = dividend(1, 2025, 2024, "", dec!(1.25));
        computed.payout_ratio_cash = dec!(77.1605);
        computed.payout_ratio = dec!(77.1605);
        computed.payout_eps = Some(dec!(1.62));
        computed.payout_period = Some("2024".to_string());
        let mut etf = dividend(2, 2025, 2024, "", dec!(1));
        etf.security_code = "0056".to_string();

        let result = calculate_payout_ratios(&[computed, etf], &hao_zhan_earnings());

        assert!(result.is_empty(), "{result:?}");
    }

    /// 配股也各自算分配率：現金 7.5、配股 0.5，涵蓋期間 EPS 6.58（2753 的 25H2）。
    #[test]
    fn splits_cash_and_stock_ratios() {
        let mut row = dividend(1, 2026, 2025, "H2", dec!(7.5));
        row.security_code = "2753".to_string();
        row.stock_dividend = dec!(0.5);
        row.sum = dec!(8);
        let earnings = vec![
            PeriodEarnings {
                security_code: "2753".to_string(),
                year: 2025,
                quarter: "Q3".to_string(),
                earnings_per_share: dec!(3.2),
            },
            PeriodEarnings {
                security_code: "2753".to_string(),
                year: 2025,
                quarter: "Q4".to_string(),
                earnings_per_share: dec!(3.38),
            },
        ];
        // H2 前面沒有 H1 配息時會往前涵蓋整年；這裡補上 H1 讓 H2 只涵蓋下半年。
        let mut first_half = dividend(2, 2025, 2025, "H1", dec!(1));
        first_half.security_code = "2753".to_string();

        let result = calculate_payout_ratios(&[row, first_half], &earnings);

        let second_half = find(&result, 1);
        assert_eq!(second_half.payout_ratio_cash, dec!(113.9818));
        assert_eq!(second_half.payout_ratio_stock, dec!(7.5988));
        assert_eq!(second_half.payout_ratio, dec!(121.5805));
    }

    #[test]
    fn period_label_formats() {
        let point = |year, quarter| QuarterPoint { year, quarter };
        let label = |start, end| {
            period_label(&Coverage {
                start,
                end,
                eps: Decimal::ONE,
            })
        };

        assert_eq!(label(point(2025, 1), point(2025, 4)), "2025");
        assert_eq!(label(point(2026, 2), point(2026, 2)), "2026Q2");
        assert_eq!(label(point(2026, 1), point(2026, 2)), "2026Q1~Q2");
        assert_eq!(label(point(2025, 4), point(2026, 2)), "2025Q4~2026Q2");
    }
}
