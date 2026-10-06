//! 用交易所資料直接補 ETF 配息：交易所有、資料庫沒有、Yahoo 也補不到的 ETF 現金配息
//! （Yahoo 已下架的債券 ETF 回 404；月配 ETF 一個月除息兩次時 Yahoo 常漏列其中一次）。
//!
//! 交易所只給除息日與金額、沒有所屬期間，因此期別依配息頻率推算：
//!
//! - 頻率由歷次除息日（交易所事件與資料庫既有列）的間隔中位數判斷：45 天內為月配、
//!   135 天內為季配、270 天內為半年配；年配或資料不足的不補。
//! - 期別取除息日的**前一期**（月配 7 月除息 → `M06`；季配 2 月除息 → 前一年 `Q4`），
//!   與既有 ETF 資料列的慣例相同；該期已有資料列時改用除息日當期（同月兩次除息時第二次落在當月）。
//! - 發放年度取除息日的年度，12 月 10 日以後除息的視為隔年發放。
//!
//! 期別只影響顯示，CAGR 與殖利率用的是除息日與金額。為避免和 Yahoo 晚到的資料重複計入，
//! 只補除息日已過 [`SETTLE_DAYS`] 天的事件，附近有對不上交易所日期的資料列時也不補。
//! 只補股票主檔中未下市的 ETF；已下市或主檔沒有的代號不建立資料列。

use std::collections::{BTreeSet, HashMap, HashSet};

use anyhow::{Context, Result};
use chrono::{Datelike, Local, NaiveDate};
use rust_decimal::Decimal;

use super::{DATE_FORMAT, parse_date, plan::event_rows};
use crate::{
    app::calculation::dividend_record,
    domain::dividend::{entity::Dividend, repository::DividendRepository},
    infra::{
        cache::SHARE, crawler::share::ExDividendAnnouncement,
        database::repository::dividend::PgDividendRepository,
    },
};

/// 除息日至少已過這麼多天才補，留時間給 Yahoo 列出同一筆配息。
pub(super) const SETTLE_DAYS: i64 = 30;

/// 附近資料列的檢查範圍：這段期間內有日期對不上交易所的資料列，可能是同一筆配息的舊日期。
const NEARBY_DAYS: i64 = 45;

/// 配息頻率。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cadence {
    Monthly,
    Quarterly,
    SemiAnnual,
}

/// 交易所資料的期間；期間外的日期無從判斷是否對得上交易所。
#[derive(Debug, Clone, Copy)]
pub(super) struct Window {
    pub(super) start: NaiveDate,
    pub(super) end: NaiveDate,
}

/// 從交易所資料補 ETF 配息並寫入；回傳補入的資料列。
///
/// 寫入後重算受影響的年度合計與持股已領股利。
pub(super) async fn fill_etf_from_exchange(
    repo: &PgDividendRepository,
    events: &[ExDividendAnnouncement],
    official: &[ExDividendAnnouncement],
    existing: &HashMap<String, Vec<Dividend>>,
    window: Window,
) -> Result<Vec<Dividend>> {
    let tracked: HashSet<String> = SHARE
        .stocks
        .read()
        .map(|stocks| {
            stocks
                .iter()
                .filter(|(_, stock)| !stock.suspend_listing())
                .map(|(symbol, _)| symbol.clone())
                .collect()
        })
        .unwrap_or_default();
    let tracked_events: Vec<ExDividendAnnouncement> = events
        .iter()
        .filter(|event| tracked.contains(&event.stock_symbol))
        .cloned()
        .collect();
    let fills = exchange_fills(&tracked_events, official, existing, window);
    let mut totals = BTreeSet::new();
    for dividend in &fills {
        repo.save(dividend).await.with_context(|| {
            format!(
                "Failed to save dividend filled from exchange {} {} {}",
                dividend.security_code, dividend.year, dividend.quarter
            )
        })?;
        totals.insert((dividend.security_code.as_str(), dividend.year));
    }
    for (symbol, year) in &totals {
        repo.upsert_annual_total_dividend(symbol, *year).await?;
    }
    let symbols: BTreeSet<&str> = totals.iter().map(|(symbol, _)| *symbol).collect();
    for symbol in symbols {
        dividend_record::backfill_received_dividend_records_for_stock(symbol)
            .await
            .with_context(|| format!("Failed to rebuild dividend records for {symbol}"))?;
    }
    Ok(fills)
}

/// 挑出可以用交易所資料補的 ETF 配息，組成資料列（不做 I/O）。
///
/// `events` 是資料庫缺、Yahoo 也補不到的事件；`official` 是同一輪（`window` 期間）全部的交易所事件。
pub(super) fn exchange_fills(
    events: &[ExDividendAnnouncement],
    official: &[ExDividendAnnouncement],
    existing: &HashMap<String, Vec<Dividend>>,
    window: Window,
) -> Vec<Dividend> {
    let mut official_dates: HashMap<&str, BTreeSet<NaiveDate>> = HashMap::new();
    for event in official {
        official_dates
            .entry(event.stock_symbol.as_str())
            .or_default()
            .insert(event.ex_date);
    }

    let mut chosen: Vec<Dividend> = Vec::new();
    for event in events {
        let Some(cash) = event.cash_dividend.filter(|cash| *cash > Decimal::ZERO) else {
            continue;
        };
        if !event.stock_symbol.starts_with("00")
            || !event.is_cash
            || event.is_stock
            || (window.end - event.ex_date).num_days() < SETTLE_DAYS
        {
            continue;
        }
        let official = official_dates
            .get(event.stock_symbol.as_str())
            .cloned()
            .unwrap_or_default();
        let rows = event_rows(existing, &event.stock_symbol);
        let mut known_dates = official.clone();
        known_dates.extend(
            rows.iter()
                .filter_map(|row| parse_date(&row.ex_dividend_date_cash)),
        );
        let Some(cadence) = cadence(&known_dates) else {
            continue;
        };
        if has_nearby_unmatched_row(&rows, event, cash, &official, window) {
            continue;
        }

        let year = payment_year(event.ex_date);
        let occupied = |year_of_dividend: i32, quarter: &str| {
            rows.iter()
                .map(|row| (row.year, row.year_of_dividend, row.quarter.as_str()))
                .chain(
                    chosen
                        .iter()
                        .filter(|picked| picked.security_code == event.stock_symbol)
                        .map(|picked| {
                            (
                                picked.year,
                                picked.year_of_dividend,
                                picked.quarter.as_str(),
                            )
                        }),
                )
                .any(|key| key == (year, year_of_dividend, quarter))
        };
        let Some((year_of_dividend, quarter)) = [true, false]
            .into_iter()
            .map(|previous| period(cadence, event.ex_date, previous))
            .find(|(year_of_dividend, quarter)| !occupied(*year_of_dividend, quarter))
        else {
            continue;
        };

        chosen.push(new_dividend(event, cash, year, year_of_dividend, quarter));
    }
    chosen
}

/// 由交易所歷次除息日間隔的中位數判斷配息頻率；少於兩次或年配時回傳 `None`。
fn cadence(dates: &BTreeSet<NaiveDate>) -> Option<Cadence> {
    let mut gaps: Vec<i64> = dates
        .iter()
        .zip(dates.iter().skip(1))
        .map(|(earlier, later)| (*later - *earlier).num_days())
        .collect();
    if gaps.is_empty() {
        return None;
    }
    gaps.sort_unstable();
    match gaps[gaps.len() / 2] {
        ..=45 => Some(Cadence::Monthly),
        46..=135 => Some(Cadence::Quarterly),
        136..=270 => Some(Cadence::SemiAnnual),
        _ => None,
    }
}

/// 附近是否有可能是同一筆配息、但日期對不上交易所的資料列：
/// 現金除息日在 [`NEARBY_DAYS`] 天內、落在交易所資料期間內卻不是交易所日期，
/// 或日期未公布但金額相近。
fn has_nearby_unmatched_row(
    rows: &[&Dividend],
    event: &ExDividendAnnouncement,
    cash: Decimal,
    official_dates: &BTreeSet<NaiveDate>,
    window: Window,
) -> bool {
    rows.iter()
        .any(|row| match parse_date(&row.ex_dividend_date_cash) {
            Some(date) => {
                (window.start..=window.end).contains(&date)
                    && !official_dates.contains(&date)
                    && (date - event.ex_date).num_days().abs() <= NEARBY_DAYS
            }
            None => {
                row.cash_dividend > Decimal::ZERO
                    && (row.year == event.ex_date.year() || row.year == event.ex_date.year() + 1)
                    && ((row.cash_dividend - cash).abs() / cash) <= super::plan::AMOUNT_TOLERANCE
            }
        })
}

/// 發放年度：12 月 10 日以後除息的，現金多在隔年 1 月發放。
fn payment_year(ex_date: NaiveDate) -> i32 {
    if ex_date.month() == 12 && ex_date.day() >= 10 {
        ex_date.year() + 1
    } else {
        ex_date.year()
    }
}

/// 除息日對應的期別 `(所屬年度, 期別)`；`previous` 為 true 時取前一期。
fn period(cadence: Cadence, ex_date: NaiveDate, previous: bool) -> (i32, String) {
    let year = ex_date.year();
    let (per_year, index) = match cadence {
        Cadence::Monthly => (12, ex_date.month()),
        Cadence::Quarterly => (4, (ex_date.month() - 1) / 3 + 1),
        Cadence::SemiAnnual => (2, (ex_date.month() - 1) / 6 + 1),
    };
    let (year, index) = match (previous, index) {
        (true, 1) => (year - 1, per_year),
        (true, index) => (year, index - 1),
        (false, index) => (year, index),
    };
    let quarter = match cadence {
        Cadence::Monthly => format!("M{index:02}"),
        Cadence::Quarterly => format!("Q{index}"),
        Cadence::SemiAnnual => format!("H{index}"),
    };
    (year, quarter)
}

/// 以交易所的除息日與金額組成資料列；盈餘、公積的拆分與發放日交易所沒有，維持 0 與 `-`。
fn new_dividend(
    event: &ExDividendAnnouncement,
    cash: Decimal,
    year: i32,
    year_of_dividend: i32,
    quarter: String,
) -> Dividend {
    let now = Local::now();
    Dividend {
        serial: 0,
        year,
        year_of_dividend,
        quarter,
        security_code: event.stock_symbol.clone(),
        earnings_cash_dividend: Decimal::ZERO,
        capital_reserve_cash_dividend: Decimal::ZERO,
        cash_dividend: cash,
        earnings_stock_dividend: Decimal::ZERO,
        capital_reserve_stock_dividend: Decimal::ZERO,
        stock_dividend: Decimal::ZERO,
        sum: cash,
        payout_ratio_cash: Decimal::ZERO,
        payout_ratio_stock: Decimal::ZERO,
        payout_ratio: Decimal::ZERO,
        ex_dividend_date_cash: event.ex_date.format(DATE_FORMAT).to_string(),
        ex_dividend_date_stock: "-".to_string(),
        payable_date_cash: "-".to_string(),
        payable_date_stock: "-".to_string(),
        created_time: now,
        updated_time: now,
    }
}

/// 補入後仍沒有資料列的事件（`代號 除權息日`）。
pub(super) fn unresolved_after_exchange_fill(
    events: &[ExDividendAnnouncement],
    fills: &[Dividend],
) -> Vec<String> {
    events
        .iter()
        .filter(|event| {
            let date = event.ex_date.format(DATE_FORMAT).to_string();
            !fills.iter().any(|fill| {
                fill.security_code == event.stock_symbol && fill.ex_dividend_date_cash == date
            })
        })
        .map(super::event_label)
        .collect()
}
