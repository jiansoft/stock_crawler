//! # 測試共用夾具
//!
//! 各子模組的單元測試都需要組出公告、分派情形、資料庫資料列等假資料，
//! 這裡集中提供建構函式，避免同一份夾具在多個檔案裡各寫一次。
//!
//! 只在 `cfg(test)` 下編譯，不會進入正式產物。

use std::collections::HashMap;

use chrono::{Local, NaiveDate, TimeZone};
use rust_decimal::Decimal;

use crate::{
    core::declare::StockExchangeMarket,
    domain::dividend::entity::Dividend,
    infra::crawler::{
        moneydj::dividend_schedule::DividendSchedule, mops::dividend_allotment::DividendAllotment,
        share::ExDividendAnnouncement, yahoo::dividend::YahooDividendDetail,
    },
};

use super::{UNANNOUNCED_DATE, quarter::ResolvedDividend};

pub(super) fn announcement(
    symbol: &str,
    ex_date: (i32, u32, u32),
    is_cash: bool,
    is_stock: bool,
    cash: Option<Decimal>,
    stock_ratio: Option<Decimal>,
) -> ExDividendAnnouncement {
    ExDividendAnnouncement {
        stock_symbol: symbol.to_string(),
        name: symbol.to_string(),
        ex_date: NaiveDate::from_ymd_opt(ex_date.0, ex_date.1, ex_date.2).unwrap(),
        is_cash,
        is_stock,
        cash_dividend: cash,
        stock_dividend_ratio: stock_ratio,
        market: StockExchangeMarket::Listed,
    }
}

pub(super) fn allotment(
    symbol: &str,
    year_of_dividend: i32,
    quarter: &str,
    earnings_cash: Decimal,
    earnings_stock: Decimal,
) -> DividendAllotment {
    DividendAllotment {
        stock_symbol: symbol.to_string(),
        year_of_dividend,
        quarter: quarter.to_string(),
        period_start: None,
        period_end: None,
        earnings_cash,
        capital_reserve_cash: Decimal::ZERO,
        earnings_stock,
        capital_reserve_stock: Decimal::ZERO,
        progress: "董事會決議".to_string(),
    }
}

pub(super) fn resolved(quarter: &str, cash: Decimal, paid_year: Option<i32>) -> ResolvedDividend {
    ResolvedDividend {
        year_of_dividend: 2025,
        paid_year,
        quarter: quarter.to_string(),
        cash_dividend: cash,
        stock_dividend: Decimal::ZERO,
        earnings_cash: cash,
        capital_reserve_cash: Decimal::ZERO,
        earnings_stock: Decimal::ZERO,
        capital_reserve_stock: Decimal::ZERO,
    }
}

pub(super) fn existing_row(
    serial: i64,
    symbol: &str,
    year: i32,
    quarter: &str,
    ex_cash: &str,
    payable_cash: &str,
) -> Dividend {
    let created = Local.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Dividend {
        serial,
        year,
        year_of_dividend: year - 1,
        quarter: quarter.to_string(),
        security_code: symbol.to_string(),
        earnings_cash_dividend: Decimal::ZERO,
        capital_reserve_cash_dividend: Decimal::ZERO,
        cash_dividend: Decimal::ZERO,
        earnings_stock_dividend: Decimal::ZERO,
        capital_reserve_stock_dividend: Decimal::ZERO,
        stock_dividend: Decimal::ZERO,
        sum: Decimal::ZERO,
        payout_ratio_cash: Decimal::ZERO,
        payout_ratio_stock: Decimal::ZERO,
        payout_ratio: Decimal::ZERO,
        ex_dividend_date_cash: ex_cash.to_string(),
        ex_dividend_date_stock: UNANNOUNCED_DATE.to_string(),
        payable_date_cash: payable_cash.to_string(),
        payable_date_stock: UNANNOUNCED_DATE.to_string(),
        created_time: created,
        updated_time: created,
    }
}

pub(super) fn yahoo_detail(
    year: i32,
    year_of_dividend: i32,
    quarter: &str,
    cash: Decimal,
    stock: Decimal,
    ex_cash: &str,
    ex_stock: &str,
) -> YahooDividendDetail {
    YahooDividendDetail {
        year,
        year_of_dividend,
        quarter: quarter.to_string(),
        cash_dividend: cash,
        stock_dividend: stock,
        ex_dividend_date1: ex_cash.to_string(),
        ex_dividend_date2: ex_stock.to_string(),
        payable_date1: UNANNOUNCED_DATE.to_string(),
        payable_date2: UNANNOUNCED_DATE.to_string(),
    }
}

pub(super) fn index(rows: Vec<Dividend>) -> HashMap<String, Vec<Dividend>> {
    let mut map: HashMap<String, Vec<Dividend>> = HashMap::new();
    for row in rows {
        map.entry(row.security_code.clone()).or_default().push(row);
    }
    map
}

pub(super) fn schedule(
    symbol: &str,
    ex_date: (i32, u32, u32),
    payable: (i32, u32, u32),
) -> DividendSchedule {
    DividendSchedule {
        stock_symbol: symbol.to_string(),
        name: symbol.to_string(),
        ex_date: NaiveDate::from_ymd_opt(ex_date.0, ex_date.1, ex_date.2).unwrap(),
        is_cash: true,
        is_stock: false,
        cash_dividend: None,
        stock_dividend: None,
        cash_payable_date: NaiveDate::from_ymd_opt(payable.0, payable.1, payable.2),
    }
}
