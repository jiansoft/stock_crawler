//! # 異動計畫的彙整
//!
//! 把每一筆除權除息公告轉成「要對資料庫做什麼」，但先不真的寫入；
//! 實際套用由 [`super::apply`] 負責，期別來源則由 [`super::quarter`] 與
//! [`super::yahoo_fallback`] 提供。
//!
//! 這裡是 [`super`] 所列第三個資料模型陷阱的實作所在：**同一次配息可能拆成兩筆公告**
//! （除息日與除權日不同天）。[`ScanPlan`] 的更新以 serial、新增以唯一鍵彙整，
//! 兩筆公告因此會累積套用到同一個實體上，而不是後者把前者補好的日期洗掉。

use std::collections::{BTreeMap, HashMap};

use chrono::NaiveDate;

use crate::{
    core::declare::StockExchangeMarket,
    domain::dividend::entity::Dividend,
    infra::crawler::{
        moneydj::dividend_schedule::DividendSchedule, mops::dividend_allotment::DividendAllotment,
        share::ExDividendAnnouncement,
    },
};

use super::{
    DATE_FORMAT,
    quarter::{
        ResolvedDividend, UnresolvedEvent, UnresolvedReason, effective_quarter, match_allotment,
        resolve_paid_year,
    },
    row::{RowMatch, build_new_dividend, match_existing_row, merge_announcement_dates},
};

/// 待新增資料列的唯一鍵：`(股票代號, 發放年度, 所屬年度, 季別)`，與資料表的主鍵一致。
type InsertKey = (String, i32, i32, String);

/// 一次掃描要對資料庫做的所有異動。
///
/// 更新以 serial、新增以唯一鍵彙整：同一次配息可能拆成除息與除權兩筆公告，
/// 兩筆都必須累積套用到同一個實體上，否則後處理的那筆會把先前補好的日期洗掉。
#[derive(Debug, Default, PartialEq)]
pub(super) struct ScanPlan {
    /// 既有資料列的日期補正；key 為 serial。
    pub(super) updates: BTreeMap<i64, Dividend>,
    /// 資料庫沒有、需要新增的事件；key 為唯一鍵。
    pub(super) inserts: BTreeMap<InsertKey, Dividend>,
    /// 無法判定期別的事件。
    pub(super) unresolved: Vec<UnresolvedEvent>,
}

/// 依採集結果與資料庫現況產生異動計畫（第一階段）。
///
/// 這是純函式，所有判斷邏輯都在這裡，方便以組好的資料做單元測試。
pub(super) fn build_scan_plan(
    announcements: &[ExDividendAnnouncement],
    allotments: &HashMap<String, Vec<DividendAllotment>>,
    schedules: &HashMap<(String, NaiveDate), DividendSchedule>,
    existing: &HashMap<String, Vec<Dividend>>,
) -> ScanPlan {
    let mut plan = ScanPlan::default();

    for announcement in announcements {
        let resolved =
            match_allotment(announcement, allotments).map(ResolvedDividend::from_allotment);
        let fallback_reason = match announcement.market {
            StockExchangeMarket::OverTheCounter => UnresolvedReason::OverTheCounterUnsupported,
            _ => UnresolvedReason::NoMatchingAllotment,
        };

        apply_event(
            &mut plan,
            announcement,
            resolved.as_ref(),
            schedules,
            existing,
            fallback_reason,
        );
    }

    plan
}

/// 把單一公告事件併入異動計畫。
///
/// 先看資料庫是否已經有這次事件；找得到就只補日期，不動金額——
/// 金額以既有採集器（Goodinfo）的拆分為準，預告表只有合計值。
/// 資料庫沒有時才需要期別，判不出來就記成待處理。
///
/// `fallback_reason` 是判不出期別時要記錄的原因，讓第一、二階段能標示不同來由。
pub(super) fn apply_event(
    plan: &mut ScanPlan,
    announcement: &ExDividendAnnouncement,
    resolved: Option<&ResolvedDividend>,
    schedules: &HashMap<(String, NaiveDate), DividendSchedule>,
    existing: &HashMap<String, Vec<Dividend>>,
    fallback_reason: UnresolvedReason,
) {
    let ex_date_text = announcement.ex_date.format(DATE_FORMAT).to_string();
    let payable_date_cash = schedules
        .get(&(announcement.stock_symbol.clone(), announcement.ex_date))
        .and_then(|schedule| schedule.cash_payable_date)
        .map(|date| date.format(DATE_FORMAT).to_string());
    let rows = existing
        .get(&announcement.stock_symbol)
        .map(Vec::as_slice)
        .unwrap_or_default();

    let paid_year = resolve_paid_year(announcement, resolved, payable_date_cash.as_deref());
    let quarter = resolved.map(|item| effective_quarter(&item.quarter, rows, paid_year));

    match match_existing_row(
        announcement,
        &ex_date_text,
        quarter.as_deref(),
        resolved,
        rows,
        paid_year,
    ) {
        RowMatch::Matched(row) => {
            // 同一批公告可能已經動過這一列（除息、除權分兩筆公告），
            // 要以計畫中的暫存值為基礎繼續套用，不能每次都從資料庫快照重來。
            let mut candidate = plan
                .updates
                .get(&row.serial)
                .cloned()
                .unwrap_or_else(|| row.clone());
            if merge_announcement_dates(
                &mut candidate,
                announcement,
                &ex_date_text,
                payable_date_cash.as_deref(),
            ) {
                plan.updates.insert(row.serial, candidate);
            }
        }
        RowMatch::Ambiguous => plan.unresolved.push(UnresolvedEvent {
            announcement: announcement.clone(),
            reason: UnresolvedReason::AmbiguousExistingRow,
        }),
        RowMatch::NotFound => {
            let (Some(resolved), Some(quarter)) = (resolved, quarter) else {
                plan.unresolved.push(UnresolvedEvent {
                    announcement: announcement.clone(),
                    reason: fallback_reason,
                });
                return;
            };

            let key = (
                announcement.stock_symbol.clone(),
                paid_year,
                resolved.year_of_dividend,
                quarter.clone(),
            );
            let entry = plan
                .inserts
                .entry(key)
                .or_insert_with(|| build_new_dividend(announcement, resolved, paid_year, &quarter));
            merge_announcement_dates(
                entry,
                announcement,
                &ex_date_text,
                payable_date_cash.as_deref(),
            );
        }
    }
}

#[cfg(test)]
mod tests;
