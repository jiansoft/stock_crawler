//! 持股股利入帳：一筆股利分配給某位會員的股數與金額，依現金、股票股利各自的發放日拆成入帳。
//!
//! 已除權息的取除權息當天寫入的已領股利明細（除權息後才賣出、尚未發放的也算）；還沒除權息的以
//! 目前未賣出持股推算並標 `estimated`。Data API `/portfolio/upcoming-payouts` 與入帳通知共用。

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

/// 一筆股利分配給一位會員的結果（現金與股票股利的發放日可能不同）。
#[derive(Debug, Clone, PartialEq)]
pub struct PayoutAllocation {
    /// 會員編號。
    pub member_id: i64,
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 季別（年度合計列為空字串）。
    pub quarter: String,
    /// 每股現金股利。
    pub cash_dividend: Decimal,
    /// 每股股票股利（元）。
    pub stock_dividend: Decimal,
    /// 現金股利發放日。
    pub cash_payable: Option<NaiveDate>,
    /// 股票股利發放日。
    pub stock_payable: Option<NaiveDate>,
    /// 除息日。
    pub cash_ex: Option<NaiveDate>,
    /// 除權日。
    pub stock_ex: Option<NaiveDate>,
    /// 參與分配的股數。
    pub shares: i64,
    /// 現金股利金額（元）。
    pub cash_amount: Decimal,
    /// 配股股數（可能有小數，入帳時取整股）。
    pub stock_shares: Decimal,
    /// 尚未除權息、以目前持股推算時為 `true`。
    pub estimated: bool,
}

/// 某一天的一筆股利入帳；同一筆股利的現金與股票股利同一天發放時合為一筆。
#[derive(Debug, Clone, PartialEq)]
pub struct DividendPayout {
    /// 會員編號；[`merge_members`] 合併後為 0。
    pub member_id: i64,
    /// 發放日。
    pub payable_date: NaiveDate,
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 季別（年度合計列為空字串）。
    pub quarter: String,
    /// 這次入帳對應的除權息日（現金取除息日、股票取除權日，缺的時候取另一個）。
    pub ex_date: Option<NaiveDate>,
    /// 參與分配的股數。
    pub shares: i64,
    /// 這天入帳的每股現金股利；這天沒有現金入帳時為 0。
    pub cash_dividend: Decimal,
    /// 這天入帳的每股股票股利；這天沒有配股入帳時為 0。
    pub stock_dividend: Decimal,
    /// 這天入帳的現金（元）。
    pub cash_amount: Decimal,
    /// 這天入帳的配股（整股）。
    pub stock_shares: i64,
    /// 以目前持股推算時為 `true`。
    pub estimated: bool,
}

/// 依現金、股票股利各自的發放日拆成入帳，只留 `from..=to` 內的；同一天的合為一筆。
///
/// 金額為 0 或配股不足一股的部分不列。結果依發放日、代號、季別、會員排序。
pub fn split_by_payable_date(
    allocations: &[PayoutAllocation],
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<DividendPayout> {
    let in_range = |date: Option<NaiveDate>| date.filter(|date| (from..=to).contains(date));
    let mut payouts = Vec::with_capacity(allocations.len());
    for allocation in allocations {
        let cash =
            in_range(allocation.cash_payable).filter(|_| allocation.cash_amount > Decimal::ZERO);
        let stock_shares = allocation.stock_shares.floor().to_i64().unwrap_or_default();
        let stock = in_range(allocation.stock_payable).filter(|_| stock_shares > 0);
        if let Some(date) = cash {
            let mut payout =
                empty_payout(allocation, date, allocation.cash_ex.or(allocation.stock_ex));
            payout.cash_dividend = allocation.cash_dividend;
            payout.cash_amount = allocation.cash_amount;
            if stock == Some(date) {
                payout.stock_dividend = allocation.stock_dividend;
                payout.stock_shares = stock_shares;
            }
            payouts.push(payout);
        }
        if let Some(date) = stock.filter(|date| cash != Some(*date)) {
            let mut payout =
                empty_payout(allocation, date, allocation.stock_ex.or(allocation.cash_ex));
            payout.stock_dividend = allocation.stock_dividend;
            payout.stock_shares = stock_shares;
            payouts.push(payout);
        }
    }
    payouts.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
    payouts
}

/// 把不同會員同一天、同一筆股利的入帳加總成一筆（`member_id` 為 0），排序同
/// [`split_by_payable_date`]。
pub fn merge_members(payouts: Vec<DividendPayout>) -> Vec<DividendPayout> {
    let mut merged: Vec<DividendPayout> = Vec::with_capacity(payouts.len());
    for mut payout in payouts {
        payout.member_id = 0;
        match merged
            .iter_mut()
            .find(|existing| same_payout(existing, &payout))
        {
            Some(existing) => {
                existing.shares += payout.shares;
                existing.cash_amount += payout.cash_amount;
                existing.stock_shares += payout.stock_shares;
            }
            None => merged.push(payout),
        }
    }
    merged.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
    merged
}

/// 入帳現金合計。
pub fn total_cash(payouts: &[DividendPayout]) -> Decimal {
    payouts.iter().map(|payout| payout.cash_amount).sum()
}

/// 金額與股利都是 0 的入帳骨架，呼叫端再填這天入帳的部分。
fn empty_payout(
    allocation: &PayoutAllocation,
    payable_date: NaiveDate,
    ex_date: Option<NaiveDate>,
) -> DividendPayout {
    DividendPayout {
        member_id: allocation.member_id,
        payable_date,
        stock_symbol: allocation.stock_symbol.clone(),
        name: allocation.name.clone(),
        quarter: allocation.quarter.clone(),
        ex_date,
        shares: allocation.shares,
        cash_dividend: Decimal::ZERO,
        stock_dividend: Decimal::ZERO,
        cash_amount: Decimal::ZERO,
        stock_shares: 0,
        estimated: allocation.estimated,
    }
}

/// 同一天、同一筆股利、同樣是（或不是）推算的入帳。
fn same_payout(a: &DividendPayout, b: &DividendPayout) -> bool {
    a.payable_date == b.payable_date
        && a.stock_symbol == b.stock_symbol
        && a.quarter == b.quarter
        && a.estimated == b.estimated
        && a.cash_dividend == b.cash_dividend
        && a.stock_dividend == b.stock_dividend
}

fn sort_key(payout: &DividendPayout) -> (NaiveDate, &str, &str, i64) {
    (
        payout.payable_date,
        payout.stock_symbol.as_str(),
        payout.quarter.as_str(),
        payout.member_id,
    )
}

#[cfg(test)]
mod tests;
