//! 已除息、有已領股利明細的現金股利：股數與金額取明細（除息日當時的持股），不用目前持股。

use std::collections::HashMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::{Basis, ForecastItem};
use crate::domain::portfolio::payout::PayoutAllocation;

/// 已除息、已有已領股利明細的一筆現金股利（各會員合計）。
#[derive(Debug, Clone, PartialEq)]
pub struct Entitlement {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 除息日。
    pub ex_date: NaiveDate,
    /// 發放日。
    pub pay_date: NaiveDate,
    /// 每股現金股利。
    pub cash_per_share: Decimal,
    /// 除息日參與分配的股數。
    pub shares: i64,
    /// 已領股利明細的現金金額。
    pub amount: Decimal,
}

impl Entitlement {
    pub(super) fn to_item(&self) -> ForecastItem {
        ForecastItem {
            stock_symbol: self.stock_symbol.clone(),
            name: self.name.clone(),
            shares: self.shares,
            ex_date: self.ex_date,
            pay_date: self.pay_date,
            cash_per_share: self.cash_per_share,
            amount: self.amount,
            basis: Basis::Announced,
            entitled: true,
        }
    }
}

/// 由股利入帳分配取出已除息、有已領明細的現金股利，依（代號, 除息日）合併各會員。
///
/// 以目前持股推算（`estimated`）、沒有現金、缺除息日或發放日的列不取；同一除息日有多列
/// （例如月配 ETF 的季別與月別）時取金額大者，與 [`forecast`](super::forecast) 對股利列的去重一致。
pub fn entitlements(allocations: &[PayoutAllocation]) -> Vec<Entitlement> {
    let mut by_row: HashMap<(&str, NaiveDate, &str), Entitlement> = HashMap::new();
    for allocation in allocations {
        if allocation.estimated || allocation.cash_amount <= Decimal::ZERO {
            continue;
        }
        let (Some(ex_date), Some(pay_date)) = (allocation.cash_ex, allocation.cash_payable) else {
            continue;
        };
        let key = (
            allocation.stock_symbol.as_str(),
            ex_date,
            allocation.quarter.as_str(),
        );
        let entry = by_row.entry(key).or_insert_with(|| Entitlement {
            stock_symbol: allocation.stock_symbol.clone(),
            name: allocation.name.clone(),
            ex_date,
            pay_date,
            cash_per_share: allocation.cash_dividend,
            shares: 0,
            amount: Decimal::ZERO,
        });
        entry.shares += allocation.shares;
        entry.amount += allocation.cash_amount;
    }
    let mut rows: Vec<Entitlement> = by_row.into_values().collect();
    rows.sort_by(|a, b| {
        (&a.stock_symbol, a.ex_date)
            .cmp(&(&b.stock_symbol, b.ex_date))
            .then_with(|| b.amount.cmp(&a.amount))
    });
    rows.dedup_by(|later, kept| {
        later.stock_symbol == kept.stock_symbol && later.ex_date == kept.ex_date
    });
    rows
}
