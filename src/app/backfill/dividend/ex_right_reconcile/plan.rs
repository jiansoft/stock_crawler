//! 比對規則：把交易所除權息事件對應到資料庫的股利列，產生要寫入的異動（不做 I/O）。

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;

use super::{DATE_FORMAT, ReconcilePlan, parse_date};
use crate::{domain::dividend::entity::Dividend, infra::crawler::share::ExDividendAnnouncement};

/// 日期延後時，資料列日期與交易所日期最多相差的天數。
pub(super) const SHIFT_WINDOW_DAYS: i64 = 60;
/// 資料列被交易所事件用掉的那一側。
///
/// 年度股利的除息與除權常在不同天（4549 桓達 2022 年 06-28 除息、08-24 除權），
/// 兩次交易所事件要能分別對上同一列；只記 serial 會讓先對上的現金事件把整列占走，
/// 除權事件就被誤判成缺漏。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Side {
    Cash,
    Stock,
}

/// 事件涉及的資料列側：除息用現金側、除權用股票側。
fn sides(event: &ExDividendAnnouncement) -> impl Iterator<Item = Side> {
    [(event.is_cash, Side::Cash), (event.is_stock, Side::Stock)]
        .into_iter()
        .filter_map(|(involved, side)| involved.then_some(side))
}

/// 資料列在這個事件涉及的每一側都還沒被其他事件用掉。
fn is_free(used: &HashSet<(i64, Side)>, row: &Dividend, event: &ExDividendAnnouncement) -> bool {
    sides(event).all(|side| !used.contains(&(row.serial, side)))
}

/// 記錄資料列被這個事件用掉的側。
fn mark_used(used: &mut HashSet<(i64, Side)>, row: &Dividend, event: &ExDividendAnnouncement) {
    used.extend(sides(event).map(|side| (row.serial, side)));
}

/// 判定「金額相近」的相對差距上限。
pub(super) const AMOUNT_TOLERANCE: Decimal = Decimal::from_parts(2, 0, 0, false, 1);
/// 日期未公布時，金額必須幾乎相同（相對差距 1% 以內）才敢對應。
pub(super) const UNANNOUNCED_TOLERANCE: Decimal = Decimal::from_parts(1, 0, 0, false, 2);

/// 依「同日 → 日期延後 → 日期未公布」的順序，把交易所事件對應到資料列並產生異動。
///
/// `as_of` 是交易所資料的截止日：資料列日期晚於它的是尚未發生的事件，不當成舊日期。
pub(crate) fn build_plan(
    official: &[ExDividendAnnouncement],
    existing: &HashMap<String, Vec<Dividend>>,
    as_of: NaiveDate,
) -> ReconcilePlan {
    let mut plan = ReconcilePlan::default();
    let mut used: HashSet<(i64, Side)> = HashSet::new();
    let mut pending: Vec<&ExDividendAnnouncement> = Vec::new();

    // 每檔股票的交易所除權息日，用來判斷資料列的日期是不是「舊日期」。
    let mut official_dates: HashMap<&str, HashSet<String>> = HashMap::new();
    for event in official {
        official_dates
            .entry(event.stock_symbol.as_str())
            .or_default()
            .insert(event.ex_date.format(DATE_FORMAT).to_string());
    }

    for event in official {
        let rows = event_rows(existing, &event.stock_symbol);
        let date = event.ex_date.format(DATE_FORMAT).to_string();
        let same_day: Vec<&Dividend> = rows
            .iter()
            .filter(|row| {
                is_free(&used, row, event)
                    && ((event.is_cash && row.ex_dividend_date_cash == date)
                        || (event.is_stock && row.ex_dividend_date_stock == date))
            })
            .copied()
            .collect();
        match same_day.as_slice() {
            [] => pending.push(event),
            [row] => {
                mark_used(&mut used, row, event);
                plan.matched += 1;
                let mut candidate = (*row).clone();
                let changed = apply_official(&mut candidate, event);
                note_stock_unknown(&mut plan, &candidate, event);
                if changed {
                    plan.updates.insert(candidate.serial, candidate);
                }
            }
            // 同一天多次配息：交易所息值是合計，逐列的金額無從拆分，只確認已收錄。
            rows_on_day => {
                for row in rows_on_day {
                    mark_used(&mut used, row, event);
                }
                plan.matched += 1;
            }
        }
    }

    for event in pending {
        let rows = event_rows(existing, &event.stock_symbol);
        let stale_dates = official_dates
            .get(event.stock_symbol.as_str())
            .cloned()
            .unwrap_or_default();

        let is_etf = event.stock_symbol.starts_with("00");
        let shifted: Vec<&Dividend> = rows
            .iter()
            .filter(|_| !is_etf)
            .filter(|row| is_free(&used, row, event))
            .filter(|row| {
                event_dates(row, event).iter().any(|date| {
                    parse_date(date).is_some_and(|row_date| {
                        row_date <= as_of
                            && !stale_dates.contains(*date)
                            && (row_date - event.ex_date).num_days().abs() <= SHIFT_WINDOW_DAYS
                    })
                })
            })
            .filter(|row| amounts_close(row, event, AMOUNT_TOLERANCE, true))
            .copied()
            .collect();

        let candidates: Vec<&Dividend> = if shifted.is_empty() {
            rows.iter()
                .filter(|row| is_free(&used, row, event))
                .filter(|row| {
                    event_dates(row, event)
                        .iter()
                        .all(|date| parse_date(date).is_none())
                })
                .filter(|row| {
                    row.year == event.ex_date.year() || row.year == event.ex_date.year() + 1
                })
                .filter(|row| amounts_close(row, event, UNANNOUNCED_TOLERANCE, false))
                .copied()
                .collect()
        } else {
            shifted
        };

        match candidates.as_slice() {
            [row] => {
                mark_used(&mut used, row, event);
                let mut candidate = (*row).clone();
                set_event_dates(&mut candidate, event);
                apply_official(&mut candidate, event);
                note_stock_unknown(&mut plan, &candidate, event);
                plan.updates.insert(candidate.serial, candidate);
            }
            [] => {
                // 證交所單純「權」可能是現金增資，對不上不算缺漏。
                let rights_only = !event.is_cash && event.stock_dividend_ratio.is_none();
                if !rights_only {
                    plan.missing.push(event.clone());
                }
            }
            _ => plan.ambiguous.push(event.clone()),
        }
    }

    plan.used = used.into_iter().map(|(serial, _)| serial).collect();
    plan
}

/// 一檔股票可參與比對的資料列：排除年度合計列（混合配息年度的空期別）。
pub(super) fn event_rows<'a>(
    existing: &'a HashMap<String, Vec<Dividend>>,
    symbol: &str,
) -> Vec<&'a Dividend> {
    let rows = existing.get(symbol).map(Vec::as_slice).unwrap_or_default();
    rows.iter()
        .filter(|row| {
            !(row.quarter.is_empty()
                && rows
                    .iter()
                    .any(|other| other.year == row.year && !other.quarter.is_empty()))
        })
        .collect()
}

/// 交易所事件類型對應到資料列的日期欄：除息看現金欄、除權看股票欄。
pub(super) fn event_dates<'a>(
    row: &'a Dividend,
    event: &ExDividendAnnouncement,
) -> Vec<&'a String> {
    let mut dates = Vec::with_capacity(2);
    if event.is_cash {
        dates.push(&row.ex_dividend_date_cash);
    }
    if event.is_stock {
        dates.push(&row.ex_dividend_date_stock);
    }
    dates
}

/// 金額是否相近：只比交易所有提供的金額；`allow_zero` 時資料列為 0 也算相近。
pub(super) fn amounts_close(
    row: &Dividend,
    event: &ExDividendAnnouncement,
    tolerance: Decimal,
    allow_zero: bool,
) -> bool {
    let close = |stored: Decimal, official: Decimal| {
        (allow_zero && stored.is_zero())
            || (official > Decimal::ZERO && ((stored - official).abs() / official) <= tolerance)
    };
    let cash_ok = event
        .cash_dividend
        .is_none_or(|official| close(row.cash_dividend, official));
    let stock_ok = event
        .stock_dividend()
        .is_none_or(|official| close(row.stock_dividend, official));
    // 交易所沒給任何金額（證交所「權」「權息」）時，未公布日期的列無從比對，只接受日期延後。
    let has_amount = event.cash_dividend.is_some() || event.stock_dividend_ratio.is_some();
    cash_ok && stock_ok && (has_amount || allow_zero)
}

/// 把交易所日期寫進資料列對應的日期欄。
pub(super) fn set_event_dates(row: &mut Dividend, event: &ExDividendAnnouncement) {
    let date = event.ex_date.format(DATE_FORMAT).to_string();
    if event.is_cash {
        row.ex_dividend_date_cash = date.clone();
    }
    if event.is_stock {
        row.ex_dividend_date_stock = date;
    }
}

/// 以交易所金額修正資料列；只在差距 20% 以內（或資料列為 0）時修正，回傳是否有變動。
///
/// 差太多代表可能是另一次配息被對上，寧可不改。
pub(super) fn apply_official(row: &mut Dividend, event: &ExDividendAnnouncement) -> bool {
    let mut changed = false;
    let mut update = |stored: &mut Decimal, official: Option<Decimal>| {
        let Some(official) = official.filter(|value| *value > Decimal::ZERO) else {
            return;
        };
        let close = stored.is_zero() || ((*stored - official).abs() / official) <= AMOUNT_TOLERANCE;
        if close && (*stored - official).abs() > Decimal::new(5, 5) {
            *stored = official;
            changed = true;
        }
    };
    update(&mut row.cash_dividend, event.cash_dividend);
    update(
        &mut row.stock_dividend,
        event.stock_dividend().map(|value| value.round_dp(4)),
    );
    if changed {
        row.sum = row.cash_dividend + row.stock_dividend;
    }
    // 同日比對時日期已經相同；兩種股利同一天除權息時補上另一欄的日期。
    let date = event.ex_date.format(DATE_FORMAT).to_string();
    if event.is_cash
        && row.cash_dividend > Decimal::ZERO
        && parse_date(&row.ex_dividend_date_cash).is_none()
    {
        row.ex_dividend_date_cash = date.clone();
        changed = true;
    }
    if event.is_stock
        && row.stock_dividend > Decimal::ZERO
        && parse_date(&row.ex_dividend_date_stock).is_none()
    {
        row.ex_dividend_date_stock = date;
        changed = true;
    }
    changed
}

/// 對上的資料列配股為 0、交易所有除權卻沒給配股金額時，記下來留給 Yahoo 補。
pub(super) fn note_stock_unknown(
    plan: &mut ReconcilePlan,
    row: &Dividend,
    event: &ExDividendAnnouncement,
) {
    if event.is_stock && event.stock_dividend_ratio.is_none() && row.stock_dividend.is_zero() {
        plan.stock_unknown.push((row.serial, event.clone()));
    }
}
