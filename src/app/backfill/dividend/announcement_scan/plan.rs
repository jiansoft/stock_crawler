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

/// 待新增資料列的唯一鍵：`(股票代號, 發放年度, 季別)`，與資料表的唯一索引一致。
type InsertKey = (String, i32, String);

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
mod tests {
    use rust_decimal_macros::dec;

    use super::super::fixtures::{
        allotment, announcement, existing_row, index, resolved, schedule,
    };
    use super::super::source::{index_allotments, index_schedules};
    use super::super::{FULL_YEAR_EVENT_QUARTER, UNANNOUNCED_DATE};
    use super::*;

    /// 資料庫已有該次事件、但發放日還是「尚未公布」時，只補日期不新增資料列。
    #[test]
    fn test_build_scan_plan_updates_payable_date_only() {
        let announcements = vec![announcement(
            "2330",
            (2026, 9, 17),
            true,
            false,
            Some(dec!(5.0)),
            None,
        )];
        let existing = index(vec![existing_row(
            11,
            "2330",
            2026,
            "Q1",
            "2026-09-17",
            UNANNOUNCED_DATE,
        )]);
        let schedules = index_schedules(vec![schedule("2330", (2026, 9, 17), (2026, 10, 8))]);

        let plan = build_scan_plan(&announcements, &HashMap::new(), &schedules, &existing);

        assert!(plan.inserts.is_empty());
        assert!(plan.unresolved.is_empty());
        assert_eq!(plan.updates.len(), 1);
        let update = &plan.updates[&11];
        assert_eq!(update.payable_date_cash, "2026-10-08");
        // 除息事件不能把除權日蓋掉。
        assert_eq!(update.ex_dividend_date_stock, UNANNOUNCED_DATE);
    }

    /// 所有日期都已經正確時不該產生任何異動。
    #[test]
    fn test_build_scan_plan_skips_when_nothing_changed() {
        let announcements = vec![announcement(
            "2330",
            (2026, 9, 17),
            true,
            false,
            Some(dec!(5.0)),
            None,
        )];
        let existing = index(vec![existing_row(
            11,
            "2330",
            2026,
            "Q1",
            "2026-09-17",
            "2026-10-08",
        )]);
        let schedules = index_schedules(vec![schedule("2330", (2026, 9, 17), (2026, 10, 8))]);

        let plan = build_scan_plan(&announcements, &HashMap::new(), &schedules, &existing);

        assert_eq!(plan, ScanPlan::default());
    }

    /// 資料庫沒有的事件、且能從分派情形配對出期別時，要新增並帶上拆分金額。
    #[test]
    fn test_build_scan_plan_inserts_missing_event() {
        let announcements = vec![announcement(
            "1231",
            (2026, 8, 20),
            true,
            true,
            Some(dec!(1.5)),
            Some(dec!(0.1)),
        )];
        let allotments = index_allotments(vec![allotment("1231", 2025, "", dec!(1.5), dec!(1.0))]);

        let plan = build_scan_plan(
            &announcements,
            &allotments,
            &HashMap::new(),
            &HashMap::new(),
        );

        assert!(plan.updates.is_empty());
        assert!(plan.unresolved.is_empty());
        assert_eq!(plan.inserts.len(), 1);

        let inserted = plan.inserts.values().next().unwrap();
        assert_eq!(inserted.security_code, "1231");
        // 發放年度取自除權息日，所屬年度與季別取自 MOPS。
        assert_eq!(inserted.year, 2026);
        assert_eq!(inserted.year_of_dividend, 2025);
        assert_eq!(inserted.quarter, "");
        assert_eq!(inserted.earnings_cash_dividend, dec!(1.5));
        assert_eq!(inserted.earnings_stock_dividend, dec!(1.0));
        assert_eq!(inserted.sum, dec!(2.5));
        assert_eq!(inserted.ex_dividend_date_cash, "2026-08-20");
        assert_eq!(inserted.ex_dividend_date_stock, "2026-08-20");
        assert_eq!(inserted.payable_date_cash, UNANNOUNCED_DATE);
    }

    /// 上櫃公司在第一階段一定配不到，要標成可交給 Yahoo 補救的原因。
    #[test]
    fn test_build_scan_plan_marks_over_the_counter_reason() {
        let mut ann = announcement("6488", (2026, 8, 26), true, false, Some(dec!(10.0)), None);
        ann.market = StockExchangeMarket::OverTheCounter;

        let plan = build_scan_plan(&[ann], &HashMap::new(), &HashMap::new(), &HashMap::new());

        assert_eq!(plan.unresolved.len(), 1);
        assert_eq!(
            plan.unresolved[0].reason,
            UnresolvedReason::OverTheCounterUnsupported
        );
        assert!(plan.unresolved[0].reason.is_retryable_with_yahoo());
    }

    /// 上市但配不到分派情形（例如 ETF）同樣要能交給 Yahoo 補救。
    #[test]
    fn test_build_scan_plan_reports_unresolved_without_allotment() {
        let announcements = vec![announcement(
            "00929",
            (2026, 8, 26),
            true,
            false,
            Some(dec!(0.1)),
            None,
        )];

        let plan = build_scan_plan(
            &announcements,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        );

        assert!(plan.updates.is_empty());
        assert!(plan.inserts.is_empty());
        assert_eq!(plan.unresolved.len(), 1);
        assert_eq!(plan.unresolved[0].announcement.stock_symbol, "00929");
        assert_eq!(
            plan.unresolved[0].reason,
            UnresolvedReason::NoMatchingAllotment
        );
    }

    /// 解出期別後，若資料庫已有同期別的資料列，要走更新而不是新增。
    #[test]
    fn test_apply_event_updates_when_quarter_row_exists() {
        let ann = announcement("6488", (2026, 8, 26), true, false, Some(dec!(10.0)), None);
        let existing = index(vec![existing_row(
            42,
            "6488",
            2026,
            "H2",
            UNANNOUNCED_DATE,
            UNANNOUNCED_DATE,
        )]);
        let mut plan = ScanPlan::default();

        apply_event(
            &mut plan,
            &ann,
            Some(&resolved("H2", dec!(10.0), Some(2026))),
            &HashMap::new(),
            &existing,
            UnresolvedReason::NoMatchingYahooDividend,
        );

        assert!(plan.inserts.is_empty());
        assert_eq!(plan.updates[&42].ex_dividend_date_cash, "2026-08-26");
    }

    /// Goodinfo 已收錄同一次配息、但期別認定不同且日期未公布時，
    /// 必須靠金額比對認出是同一筆，走更新而不是新增出重複列。
    #[test]
    fn test_apply_event_matches_by_amount_when_quarter_differs() {
        let ann = announcement("1234", (2026, 8, 20), true, false, Some(dec!(2.5)), None);
        // 資料庫那筆的季別是空字串（年度），與來源判定的 H2 不同。
        let mut row = existing_row(9, "1234", 2026, "", "-", "-");
        row.cash_dividend = dec!(2.5);
        let existing = index(vec![row]);
        let mut plan = ScanPlan::default();

        apply_event(
            &mut plan,
            &ann,
            Some(&resolved("H2", dec!(2.5), Some(2026))),
            &HashMap::new(),
            &existing,
            UnresolvedReason::NoMatchingAllotment,
        );

        assert!(plan.inserts.is_empty());
        assert_eq!(plan.updates[&9].ex_dividend_date_cash, "2026-08-20");
        assert_eq!(plan.updates[&9].quarter, "");
    }

    /// 兩筆同額且都未公布日期時無從分辨，寧可不動也不新增。
    #[test]
    fn test_apply_event_reports_ambiguous_rows() {
        let ann = announcement("1234", (2026, 8, 20), true, false, Some(dec!(2.5)), None);
        let mut first = existing_row(9, "1234", 2026, "Q1", "-", "-");
        first.cash_dividend = dec!(2.5);
        let mut second = existing_row(10, "1234", 2026, "Q2", "-", "-");
        second.cash_dividend = dec!(2.5);
        let existing = index(vec![first, second]);
        let mut plan = ScanPlan::default();

        apply_event(
            &mut plan,
            &ann,
            Some(&resolved("H2", dec!(2.5), Some(2026))),
            &HashMap::new(),
            &existing,
            UnresolvedReason::NoMatchingAllotment,
        );

        assert!(plan.updates.is_empty());
        assert!(plan.inserts.is_empty());
        assert_eq!(
            plan.unresolved[0].reason,
            UnresolvedReason::AmbiguousExistingRow
        );
    }

    /// 金額相同但日期已經公布過的列，不可被當成同一次事件覆蓋。
    #[test]
    fn test_apply_event_ignores_rows_with_announced_dates() {
        let ann = announcement("1234", (2026, 8, 20), true, false, Some(dec!(2.5)), None);
        // 這一列是同年前一次配息，日期已公布，金額剛好相同。
        let mut row = existing_row(9, "1234", 2026, "H1", "2026-03-10", "2026-04-10");
        row.cash_dividend = dec!(2.5);
        let existing = index(vec![row]);
        let mut plan = ScanPlan::default();

        apply_event(
            &mut plan,
            &ann,
            Some(&resolved("H2", dec!(2.5), Some(2026))),
            &HashMap::new(),
            &existing,
            UnresolvedReason::NoMatchingAllotment,
        );

        assert!(plan.updates.is_empty());
        assert_eq!(plan.inserts.len(), 1);
        assert_eq!(plan.inserts.values().next().unwrap().quarter, "H2");
    }

    /// 混合配息年度的空季別列是年度合計，不能被當成事件寫入除權息日；
    /// 全年事件必須改用 `A`。
    #[test]
    fn test_apply_event_never_touches_annual_total_row() {
        let ann = announcement("2072", (2026, 4, 10), true, false, Some(dec!(7.0833)), None);
        let existing = index(vec![
            // 年度合計列（空季別）+ 已存在的半年配明細 → 這是混合配息年度。
            existing_row(1, "2072", 2026, "", "-", "-"),
            existing_row(2, "2072", 2026, "H1", "2026-08-28", "2026-09-29"),
        ]);
        let mut plan = ScanPlan::default();

        apply_event(
            &mut plan,
            &ann,
            Some(&resolved("", dec!(7.0833), Some(2026))),
            &HashMap::new(),
            &existing,
            UnresolvedReason::NoMatchingAllotment,
        );

        // 合計列（serial 1）不得被更新。
        assert!(plan.updates.is_empty());
        assert_eq!(plan.inserts.len(), 1);
        let inserted = plan.inserts.values().next().unwrap();
        assert_eq!(inserted.quarter, FULL_YEAR_EVENT_QUARTER);
        assert_eq!(inserted.ex_dividend_date_cash, "2026-04-10");
    }

    /// 同一次配息拆成除息、除權兩筆公告時，兩筆日期都要留下來。
    #[test]
    fn test_apply_event_merges_two_announcements_into_one_update() {
        let cash_ann = announcement("1438", (2026, 9, 9), true, false, Some(dec!(0.2)), None);
        let stock_ann = announcement("1438", (2026, 9, 16), false, true, None, Some(dec!(0.08)));
        let existing = index(vec![existing_row(
            5,
            "1438",
            2026,
            "",
            UNANNOUNCED_DATE,
            UNANNOUNCED_DATE,
        )]);
        let mut plan = ScanPlan::default();

        for ann in [&cash_ann, &stock_ann] {
            apply_event(
                &mut plan,
                ann,
                Some(&resolved("", dec!(0.2), Some(2026))),
                &HashMap::new(),
                &existing,
                UnresolvedReason::NoMatchingAllotment,
            );
        }

        assert_eq!(plan.updates.len(), 1);
        let update = &plan.updates[&5];
        // 第二筆公告不可以把第一筆補好的除息日還原成未公布。
        assert_eq!(update.ex_dividend_date_cash, "2026-09-09");
        assert_eq!(update.ex_dividend_date_stock, "2026-09-16");
    }

    /// 同一次配息拆成兩筆公告、且資料庫沒有這筆時，只能新增一列。
    #[test]
    fn test_apply_event_merges_two_announcements_into_one_insert() {
        let cash_ann = announcement("1438", (2026, 9, 9), true, false, Some(dec!(0.2)), None);
        let stock_ann = announcement("1438", (2026, 9, 16), false, true, None, Some(dec!(0.08)));
        let mut plan = ScanPlan::default();

        for ann in [&cash_ann, &stock_ann] {
            apply_event(
                &mut plan,
                ann,
                Some(&resolved("", dec!(0.2), Some(2026))),
                &HashMap::new(),
                &HashMap::new(),
                UnresolvedReason::NoMatchingAllotment,
            );
        }

        assert_eq!(plan.inserts.len(), 1);
        let inserted = plan.inserts.values().next().unwrap();
        assert_eq!(inserted.ex_dividend_date_cash, "2026-09-09");
        assert_eq!(inserted.ex_dividend_date_stock, "2026-09-16");
    }
}
