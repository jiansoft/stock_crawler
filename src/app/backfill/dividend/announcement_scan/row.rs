//! # 既有資料列的比對與資料列組裝
//!
//! [`super::plan`] 決定「這次公告要落在哪一列」時所需要的低階操作都放在這裡：
//! 在既有資料列中認人（[`match_existing_row`]）、把公告日期套到實體上
//! （[`merge_announcement_dates`]），以及資料庫沒有時組出新的資料列
//! （[`build_new_dividend`]）。
//!
//! 比對時**年度合計列一律排除**——它是 `upsert_annual_total_dividend` 聚合出來的結果，
//! 不是一次配息事件，詳見 [`super`] 的第一個資料模型陷阱。

use chrono::Local;
use rust_decimal::Decimal;

use crate::{domain::dividend::entity::Dividend, infra::crawler::share::ExDividendAnnouncement};

use super::{
    UNANNOUNCED_DATE, UNANNOUNCED_DATE_VALUES,
    quarter::{ResolvedDividend, is_annual_total_row},
};

/// 既有資料列的比對結果。
#[derive(Debug, PartialEq)]
pub(super) enum RowMatch<'a> {
    /// 找到唯一對應的資料列。
    Matched(&'a Dividend),
    /// 沒有對應的資料列。
    NotFound,
    /// 有多筆可能對應的資料列，無法分辨。
    Ambiguous,
}

/// 在既有資料列中找出這次事件對應的那一列。
///
/// 三種比對方式依序嘗試，**年度合計列一律排除**——它是聚合結果而非事件，
/// 把除權息日寫上去會讓合計列看起來像一次配息。
pub(super) fn match_existing_row<'a>(
    announcement: &ExDividendAnnouncement,
    ex_date_text: &str,
    quarter: Option<&str>,
    resolved: Option<&ResolvedDividend>,
    rows: &'a [Dividend],
    paid_year: i32,
) -> RowMatch<'a> {
    let candidates: Vec<&Dividend> = rows
        .iter()
        .filter(|row| !is_annual_total_row(row, rows))
        .collect();

    // 1. 除權息日已經寫在資料列上（先前已收錄過這次事件）。
    //    這一步不限定發放年度：既有資料可能把跨年事件記在另一年，
    //    以日期認人才不會又新增一筆重複的。
    if let Some(row) = candidates.iter().find(|row| {
        (announcement.is_cash && row.ex_dividend_date_cash == ex_date_text)
            || (announcement.is_stock && row.ex_dividend_date_stock == ex_date_text)
    }) {
        return RowMatch::Matched(row);
    }

    let (Some(quarter), Some(resolved)) = (quarter, resolved) else {
        return RowMatch::NotFound;
    };

    // 2. 發放年度與季別都相同。
    if let Some(row) = candidates
        .iter()
        .find(|row| row.year == paid_year && row.quarter == quarter)
    {
        return RowMatch::Matched(row);
    }

    // 3. 金額相同、且日期還沒公布的列。
    //
    // 這一步是防重複列的關鍵：Goodinfo 可能已經收錄了這次配息、但期別的認定
    // 與 MOPS／Yahoo 不同（例如一邊記成年度、一邊記成 H2），日期又還是「尚未公布」。
    // 金額吻合足以判定是同一次事件，改走更新既有列，把日期補上去。
    let mut matched = candidates.iter().filter(|row| {
        row.year == paid_year
            && is_unannounced(&row.ex_dividend_date_cash)
            && is_unannounced(&row.ex_dividend_date_stock)
            && row.cash_dividend == resolved.cash_dividend
            && row.stock_dividend == resolved.stock_dividend
    });

    match (matched.next(), matched.next()) {
        // 有兩筆以上完全同額又都沒公布日期時無從分辨，寧可不動，也不要新增。
        (Some(_), Some(_)) => RowMatch::Ambiguous,
        (Some(row), None) => RowMatch::Matched(row),
        _ => RowMatch::NotFound,
    }
}

/// 判斷日期欄位是否為「尚未公布」。
fn is_unannounced(value: &str) -> bool {
    UNANNOUNCED_DATE_VALUES.contains(&value)
}

/// 把一筆公告提供的日期套用到目標實體上，回傳是否真的有變動。
///
/// 只覆寫「這次公告確實涵蓋」的欄位：除息事件不會去動除權日，
/// MoneyDJ 沒有提供發放日時也保留原值，避免把已知資料洗成未公布。
/// 股票股利發放日目前沒有任何來源可用，一律維持原值。
pub(super) fn merge_announcement_dates(
    target: &mut Dividend,
    announcement: &ExDividendAnnouncement,
    ex_date_text: &str,
    payable_date_cash: Option<&str>,
) -> bool {
    let mut changed = false;

    if announcement.is_cash && target.ex_dividend_date_cash != ex_date_text {
        target.ex_dividend_date_cash = ex_date_text.to_string();
        changed = true;
    }
    if announcement.is_stock && target.ex_dividend_date_stock != ex_date_text {
        target.ex_dividend_date_stock = ex_date_text.to_string();
        changed = true;
    }
    if let Some(date) = payable_date_cash
        && target.payable_date_cash != date
    {
        target.payable_date_cash = date.to_string();
        changed = true;
    }

    changed
}

/// 以公告與已解出的期別組出要新增的股利資料。
///
/// 金額採用期別來源（MOPS 或 Yahoo）而非預告表：前者才有拆分與完整合計。
/// 日期欄位先填「尚未公布」，再由 [`merge_announcement_dates`] 依公告覆寫。
/// 盈餘分配率留白，由既有的 [`crate::app::backfill::dividend::payout_ratio`] 流程負責計算。
pub(super) fn build_new_dividend(
    announcement: &ExDividendAnnouncement,
    resolved: &ResolvedDividend,
    paid_year: i32,
    quarter: &str,
) -> Dividend {
    let now = Local::now();

    Dividend {
        serial: 0,
        year: paid_year,
        year_of_dividend: resolved.year_of_dividend,
        quarter: quarter.to_string(),
        security_code: announcement.stock_symbol.clone(),
        earnings_cash_dividend: resolved.earnings_cash,
        capital_reserve_cash_dividend: resolved.capital_reserve_cash,
        cash_dividend: resolved.cash_dividend,
        earnings_stock_dividend: resolved.earnings_stock,
        capital_reserve_stock_dividend: resolved.capital_reserve_stock,
        stock_dividend: resolved.stock_dividend,
        sum: resolved.cash_dividend + resolved.stock_dividend,
        payout_ratio_cash: Decimal::ZERO,
        payout_ratio_stock: Decimal::ZERO,
        payout_ratio: Decimal::ZERO,
        ex_dividend_date_cash: UNANNOUNCED_DATE.to_string(),
        ex_dividend_date_stock: UNANNOUNCED_DATE.to_string(),
        payable_date_cash: UNANNOUNCED_DATE.to_string(),
        payable_date_stock: UNANNOUNCED_DATE.to_string(),
        created_time: now,
        updated_time: now,
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::super::fixtures::{announcement, existing_row};
    use super::*;

    /// 公司改期時要以公告為準覆寫既有的除權息日。
    #[test]
    fn test_merge_announcement_dates_overwrites_changed_ex_date() {
        let mut row = existing_row(7, "2882", 2026, "", "2026-08-01", "2026-08-30");
        let ann = announcement("2882", (2026, 8, 15), true, false, Some(dec!(3.5)), None);

        assert!(merge_announcement_dates(&mut row, &ann, "2026-08-15", None));
        assert_eq!(row.ex_dividend_date_cash, "2026-08-15");
        // MoneyDJ 沒給發放日時保留資料庫原值，不可洗成未公布。
        assert_eq!(row.payable_date_cash, "2026-08-30");
    }
}
