use chrono::{DateTime, Local};
use rust_decimal::Decimal;

use crate::{core::util::map::Keyable, infra::crawler::goodinfo};

pub mod dividend_record_detail;
pub mod dividend_record_detail_more;
pub(crate) mod extension;
/// `Dividend` 的資料庫寫入／更新操作子模組。
mod mutation;
/// `Dividend` 的資料庫查詢操作子模組。
mod query;

#[derive(sqlx::Type, sqlx::FromRow, Debug, Clone)]
/// 股息發放日程表 原表名 dividend
pub struct Dividend {
    /// 序號
    pub serial: i64,
    /// 發放年度
    pub year: i32,
    /// 股利所屬年度
    pub year_of_dividend: i32,
    /// 發放季度
    pub quarter: String,
    /// 股票代號
    pub security_code: String,
    /// 盈餘現金股利 (Cash Dividend)
    pub earnings_cash_dividend: Decimal,
    /// 公積現金股利 (Capital Reserve)
    pub capital_reserve_cash_dividend: Decimal,
    /// 現金股利合計
    pub cash_dividend: Decimal,
    /// 盈餘股票股利 (Stock Dividend)
    pub earnings_stock_dividend: Decimal,
    /// 公積股票股利 (Capital Reserve)
    pub capital_reserve_stock_dividend: Decimal,
    /// 股票股利合計
    pub stock_dividend: Decimal,
    /// 合計股利(元)
    pub sum: Decimal,
    /// 盈餘分配率_配息(%)
    pub payout_ratio_cash: Decimal,
    /// 盈餘分配率_配股(%)
    pub payout_ratio_stock: Decimal,
    /// 盈餘分配率(%)
    pub payout_ratio: Decimal,
    /// 除息日
    pub ex_dividend_date1: String,
    /// 除權日
    pub ex_dividend_date2: String,
    /// 現金股利發放日
    pub payable_date1: String,
    /// 股票股利發放日
    pub payable_date2: String,
    /// 建立時間。
    pub created_time: DateTime<Local>,
    /// 最後更新時間。
    pub updated_time: DateTime<Local>,
}

impl Keyable for Dividend {
    fn key(&self) -> String {
        format!(
            "{}-{}-{}",
            self.security_code, self.year_of_dividend, self.quarter
        )
    }

    fn key_with_prefix(&self) -> String {
        format!("Dividend:{}", self.key())
    }
}

impl Dividend {
    /// 建立股利資料預設值。
    pub fn new() -> Self {
        Dividend {
            serial: 0,
            year: 0,
            year_of_dividend: 0,
            quarter: "".to_string(),
            security_code: Default::default(),
            earnings_cash_dividend: Default::default(),
            capital_reserve_cash_dividend: Default::default(),
            cash_dividend: Default::default(),
            earnings_stock_dividend: Default::default(),
            capital_reserve_stock_dividend: Default::default(),
            stock_dividend: Default::default(),
            sum: Default::default(),
            payout_ratio_cash: Default::default(),
            payout_ratio_stock: Default::default(),
            payout_ratio: Default::default(),
            ex_dividend_date1: "".to_string(),
            ex_dividend_date2: "".to_string(),
            payable_date1: "".to_string(),
            payable_date2: "".to_string(),
            created_time: Local::now(),
            updated_time: Local::now(),
        }
    }
}

impl Default for Dividend {
    fn default() -> Self {
        Self::new()
    }
}
/*
impl Clone for Entity {
    fn clone(&self) -> Self {
        Entity {
            serial: self.serial,
            year: self.year,
            year_of_dividend: self.year_of_dividend,
            quarter: self.quarter.to_string(),
            security_code: self.security_code.to_string(),
            earnings_cash_dividend: self.earnings_cash_dividend,
            capital_reserve_cash_dividend: self.capital_reserve_cash_dividend,
            cash_dividend: self.cash_dividend,
            earnings_stock_dividend: self.earnings_stock_dividend,
            capital_reserve_stock_dividend: self.capital_reserve_stock_dividend,
            stock_dividend: self.stock_dividend,
            sum: self.sum,
            payout_ratio_cash: self.payout_ratio_cash,
            payout_ratio_stock: self.payout_ratio_stock,
            payout_ratio: self.payout_ratio,
            ex_dividend_date1: self.ex_dividend_date1.to_string(),
            ex_dividend_date2: self.ex_dividend_date2.to_string(),
            payable_date1: self.payable_date1.to_string(),
            payable_date2: self.payable_date2.to_string(),
            create_time: self.create_time,
            update_time: self.update_time,
        }
    }
}
*/

//let entity: Entity = fs.into(); // 或者 let entity = Entity::from(fs);
impl From<goodinfo::dividend::GoodInfoDividend> for Dividend {
    fn from(d: goodinfo::dividend::GoodInfoDividend) -> Self {
        let mut e = Dividend::new();
        e.quarter = d.quarter.clone();
        e.year = d.year;
        e.year_of_dividend = d.year_of_dividend;
        e.security_code = d.stock_symbol.clone();
        e.earnings_cash_dividend = d.earnings_cash;
        e.capital_reserve_cash_dividend = d.capital_reserve_cash;
        e.cash_dividend = d.cash_dividend;
        e.earnings_stock_dividend = d.earnings_stock;
        e.capital_reserve_stock_dividend = d.capital_reserve_stock;
        e.stock_dividend = d.stock_dividend;
        e.sum = d.sum;
        e.payout_ratio_cash = d.payout_ratio_cash;
        e.payout_ratio_stock = d.payout_ratio_stock;
        e.payout_ratio = d.payout_ratio;
        e.ex_dividend_date1 = d.ex_dividend_date1.clone();
        e.ex_dividend_date2 = d.ex_dividend_date2.clone();
        e.payable_date1 = d.payable_date1.clone();
        e.payable_date2 = d.payable_date2.clone();
        e.created_time = Local::now();
        e.updated_time = Local::now();
        e
    }
}
