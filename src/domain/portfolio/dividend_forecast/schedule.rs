//! 預估用的日期與金額小工具：同日去重、月配依間隔排程、發放日推估、月份清單與金額四捨五入。

use std::collections::HashMap;

use chrono::{Datelike, Days, Months, NaiveDate};
use rust_decimal::{Decimal, prelude::ToPrimitive};

use super::{DividendEvent, PAYABLE_LAG_DAYS};

/// 有除息日的列，同一除息日只留一列：優先各期列（期別非空），再取金額大者。
pub(super) fn dedupe_dated<'a>(events: &[&'a DividendEvent]) -> Vec<&'a DividendEvent> {
    let mut best: HashMap<NaiveDate, &DividendEvent> = HashMap::new();
    for event in events {
        let Some(ex_date) = event.ex_date else {
            continue;
        };
        let rank =
            |candidate: &DividendEvent| (!candidate.quarter.is_empty(), candidate.cash_per_share);
        best.entry(ex_date)
            .and_modify(|current| {
                if rank(event) > rank(current) {
                    *current = event;
                }
            })
            .or_insert(event);
    }
    let mut dated: Vec<&DividendEvent> = best.into_values().collect();
    dated.sort_by_key(|event| event.ex_date);
    dated
}

/// 季配（`Q`）與月配（`M`）的期別；年配（空白）與半年配（`H`）不算。
pub(super) fn is_periodic(quarter: &str) -> bool {
    quarter.starts_with('Q') || quarter.starts_with('M')
}

/// 月配依間隔往後排的（除息日, 發放日），只取發放日落在 `today` 之後、`end` 以前者。
///
/// 間隔取最近兩期除息日相差的月數（至少 1 個月；只有一期時當作 1 個月），發放日沿用
/// 最近一期「除息到發放」的天數。最近一期已超過兩個間隔沒有配息時視為停配，不排。
pub(super) fn cadence_schedule(
    periodic: &[&DividendEvent],
    today: NaiveDate,
    end: NaiveDate,
) -> Vec<(NaiveDate, NaiveDate)> {
    let Some((latest, earlier)) = periodic.split_last() else {
        return Vec::new();
    };
    let Some(latest_ex) = latest.ex_date else {
        return Vec::new();
    };
    let step = earlier
        .last()
        .and_then(|previous| previous.ex_date)
        .map_or(1, |previous_ex| {
            months_between(previous_ex, latest_ex).max(1)
        });
    let stale = latest_ex
        .checked_add_months(Months::new(step * 2))
        .is_none_or(|limit| limit < today);
    if stale {
        return Vec::new();
    }
    let pay_gap = pay_date(latest) - latest_ex;

    let mut schedule = Vec::new();
    for count in 1.. {
        let Some(ex_date) = latest_ex.checked_add_months(Months::new(step * count)) else {
            break;
        };
        let pay_date = ex_date + pay_gap;
        if pay_date > end {
            break;
        }
        if pay_date > today {
            schedule.push((ex_date, pay_date));
        }
    }
    schedule
}

/// `from` 到 `to` 相差的月數（只看年月）。
pub(super) fn months_between(from: NaiveDate, to: NaiveDate) -> u32 {
    let months = (to.year() - from.year()) * 12 + to.month() as i32 - from.month() as i32;
    u32::try_from(months).unwrap_or_default()
}

/// 發放日：已公告用公告日，否則除息日後 [`PAYABLE_LAG_DAYS`] 天。
pub(super) fn pay_date(event: &DividendEvent) -> NaiveDate {
    event.payable_date.unwrap_or_else(|| {
        let ex_date = event.ex_date.unwrap_or_default();
        ex_date + Days::new(PAYABLE_LAG_DAYS)
    })
}

pub(super) fn next_year(date: NaiveDate) -> Option<NaiveDate> {
    date.checked_add_months(Months::new(12))
}

pub(super) fn month_key(date: NaiveDate) -> String {
    format!("{:04}-{:02}", date.year(), date.month())
}

/// `today` 所在月份到 `end` 所在月份（含）的每個月。
pub(super) fn month_range(today: NaiveDate, end: NaiveDate) -> Vec<String> {
    let mut months = Vec::new();
    let mut cursor = today.with_day(1).unwrap_or(today);
    while cursor <= end {
        months.push(month_key(cursor));
        match cursor.checked_add_months(Months::new(1)) {
            Some(next) => cursor = next,
            None => break,
        }
    }
    months
}

pub(super) fn money(value: Decimal) -> f64 {
    value.round_dp(2).to_f64().unwrap_or_default()
}
