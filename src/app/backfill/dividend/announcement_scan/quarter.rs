//! # 期別判定：資料模型與第一階段配對
//!
//! 預告表只有除權息日期、沒有股利所屬期間，因此期別必須另外解出來；
//! 整體策略（兩階段判定）見 [`super`] 的說明，本模組負責其中的**第一階段**：
//! 用 MOPS 的股利分派情形以「金額完全吻合」批次配對。
//!
//! 除了配對本身，這裡也放了期別相關的共用資料模型與規則：
//!
//! - [`ResolvedDividend`]：已判定期別與金額的股利內容，是「期別來源」的抽象，
//!   MOPS 與 Yahoo 兩種來源都收斂到它。
//! - [`UnresolvedReason`] / [`UnresolvedEvent`]：判不出期別時保留的原因與整筆公告，
//!   第二階段（[`super::yahoo_fallback`]）據此決定要不要再試一次 Yahoo。
//! - [`resolve_paid_year`]、[`effective_quarter`]、[`is_annual_total_row`]：
//!   對應 [`super`] 所列的前兩個資料模型陷阱——混合配息年度的空季別是「年度合計」，
//!   以及發放年度不等於除權息日的年份。

use std::collections::HashMap;

use chrono::Datelike;
use rust_decimal::Decimal;

use crate::{
    domain::dividend::entity::Dividend,
    infra::crawler::{
        mops::dividend_allotment::DividendAllotment, share::ExDividendAnnouncement, yahoo,
    },
};

use super::FULL_YEAR_EVENT_QUARTER;

/// 事件無法判定期別的原因。
///
/// 分類是為了 log 的訊噪比與後續處理方式：能靠 Yahoo 補救的與真的沒救的要分開看。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum UnresolvedReason {
    /// 上櫃公司沒有對應的開放股利分派情形（櫃買中心未提供彙總資料）。
    OverTheCounterUnsupported,
    /// 上市公司找不到金額吻合的分派情形。
    ///
    /// 常見於 ETF 的收益分配（不適用公司股利分派）、公司尚未申報，
    /// 或同一年有兩期金額完全相同而無法分辨。
    NoMatchingAllotment,
    /// Yahoo 的股利政策裡找不到除權息日相符的紀錄。
    NoMatchingYahooDividend,
    /// Yahoo 頁面抓取失敗（含 404、連線失敗與重試耗盡）。
    YahooLookupFailed,
    /// 本次掃描的 Yahoo 查詢額度已用完，留待下次排程處理。
    YahooLookupSkipped,
    /// 資料庫中有多筆金額相同、日期未公布的列，無法確定對應哪一筆。
    AmbiguousExistingRow,
}

impl UnresolvedReason {
    /// 供 log 使用的說明文字。
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::OverTheCounterUnsupported => "上櫃無開放的股利分派情形",
            Self::NoMatchingAllotment => "找不到金額吻合的股利分派情形",
            Self::NoMatchingYahooDividend => "Yahoo 股利政策無除權息日相符的紀錄",
            Self::YahooLookupFailed => "Yahoo 股利政策抓取失敗",
            Self::YahooLookupSkipped => "本次 Yahoo 查詢額度已用完",
            Self::AmbiguousExistingRow => "資料庫有多筆金額相同且日期未公布的列",
        }
    }

    /// 是否還能靠逐檔查詢 Yahoo 補救。
    ///
    /// 第一階段（MOPS 配對）失敗的兩種原因都值得再試一次 Yahoo；
    /// 已經試過 Yahoo 的則不再重複。
    pub(super) fn is_retryable_with_yahoo(self) -> bool {
        matches!(
            self,
            Self::OverTheCounterUnsupported | Self::NoMatchingAllotment
        )
    }
}

/// 無法對應到期別、因此不敢寫入的事件。
///
/// 保留整筆公告而不只是代號，第二階段才有足夠資訊組出要寫入的資料列。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct UnresolvedEvent {
    /// 來源公告。
    pub(super) announcement: ExDividendAnnouncement,
    /// 無法處理的原因。
    pub(super) reason: UnresolvedReason,
}

/// 已判定期別與金額的股利內容。
///
/// 這是「期別來源」的抽象：不論來自 MOPS 的分派情形還是 Yahoo 的股利政策，
/// 後續組資料列的流程都一樣。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolvedDividend {
    /// 股利所屬年度（西元）。
    pub(super) year_of_dividend: i32,
    /// 來源已判定的發放年度；MOPS 沒有這項資訊時為 `None`。
    pub(super) paid_year: Option<i32>,
    /// 所屬季度：空字串為年度，`Q1`～`Q4`、`H1`／`H2`，`A` 為混合配息年度的全年事件。
    pub(super) quarter: String,
    /// 現金股利合計。
    pub(super) cash_dividend: Decimal,
    /// 股票股利合計。
    pub(super) stock_dividend: Decimal,
    /// 盈餘現金股利；無拆分來源時為 0。
    pub(super) earnings_cash: Decimal,
    /// 公積現金股利；無拆分來源時為 0。
    pub(super) capital_reserve_cash: Decimal,
    /// 盈餘股票股利；無拆分來源時為 0。
    pub(super) earnings_stock: Decimal,
    /// 公積股票股利；無拆分來源時為 0。
    pub(super) capital_reserve_stock: Decimal,
}

impl ResolvedDividend {
    /// 由 MOPS 的股利分派情形建立，帶完整的盈餘／公積拆分。
    ///
    /// MOPS 只揭露股利所屬期間，沒有發放年度，因此 `paid_year` 為 `None`，
    /// 交由 [`resolve_paid_year`] 依發放日或除權息日推定。
    pub(super) fn from_allotment(allotment: &DividendAllotment) -> Self {
        Self {
            year_of_dividend: allotment.year_of_dividend,
            paid_year: None,
            quarter: allotment.quarter.clone(),
            cash_dividend: allotment.cash_dividend(),
            stock_dividend: allotment.stock_dividend(),
            earnings_cash: allotment.earnings_cash,
            capital_reserve_cash: allotment.capital_reserve_cash,
            earnings_stock: allotment.earnings_stock,
            capital_reserve_stock: allotment.capital_reserve_stock,
        }
    }

    /// 由 Yahoo 的單次配息紀錄建立；Yahoo 沒有拆分資料，拆分欄位留 0。
    ///
    /// Yahoo 本身就是依發放年度分組的，直接沿用它的年度，
    /// 才不會與既有回補流程對同一次配息各自寫出不同年度的資料列。
    pub(super) fn from_yahoo(detail: &yahoo::dividend::YahooDividendDetail) -> Self {
        Self {
            year_of_dividend: detail.year_of_dividend,
            paid_year: Some(detail.year),
            quarter: detail.quarter.clone(),
            cash_dividend: detail.cash_dividend,
            stock_dividend: detail.stock_dividend,
            earnings_cash: Decimal::ZERO,
            capital_reserve_cash: Decimal::ZERO,
            earnings_stock: Decimal::ZERO,
            capital_reserve_stock: Decimal::ZERO,
        }
    }
}

/// 決定這次事件應該落在哪一個發放年度。
///
/// 順序刻意與既有回補流程一致，避免同一次配息在不同流程被寫成不同年度：
/// 1. Yahoo 已依發放年度分組，直接沿用。
/// 2. 季配／半年配以現金發放日的年份為準（跨年發放時與除權息日不同年）。
/// 3. 其餘退回除權息日的年份。
pub(super) fn resolve_paid_year(
    announcement: &ExDividendAnnouncement,
    resolved: Option<&ResolvedDividend>,
    payable_date_cash: Option<&str>,
) -> i32 {
    if let Some(year) = resolved.and_then(|item| item.paid_year) {
        return year;
    }

    if resolved.is_some_and(|item| !item.quarter.is_empty())
        && let Some(year) = payable_date_cash.and_then(parse_year)
    {
        return year;
    }

    announcement.ex_date.year()
}

/// 從 `YYYY-MM-DD` 取出年份。
fn parse_year(date: &str) -> Option<i32> {
    date.split('-').next()?.parse::<i32>().ok()
}

/// 判斷指定發放年度是否為「混合配息年度」（已有季配或半年配明細）。
///
/// 這種年度的空季別資料列是 `upsert_annual_total_dividend` 聚合出來的**年度合計**，
/// 不是一次真實的配息事件。
fn is_mixed_dividend_year(rows: &[Dividend], year: i32) -> bool {
    rows.iter().any(|row| {
        row.year == year && !row.quarter.is_empty() && row.quarter != FULL_YEAR_EVENT_QUARTER
    })
}

/// 判斷某一列是否為年度合計列。
pub(super) fn is_annual_total_row(row: &Dividend, rows: &[Dividend]) -> bool {
    row.quarter.is_empty() && is_mixed_dividend_year(rows, row.year)
}

/// 決定事件實際要用的季別代碼。
///
/// 來源判定為「年度」（空季別）時，若該發放年度同時有季配／半年配明細，
/// 空季別已經被年度合計列占用，全年事件必須改用 `A`，
/// 與 [`crate::infra::crawler::yahoo::dividend`] 的規則一致。
pub(super) fn effective_quarter(quarter: &str, rows: &[Dividend], paid_year: i32) -> String {
    if quarter.is_empty() && is_mixed_dividend_year(rows, paid_year) {
        return FULL_YEAR_EVENT_QUARTER.to_string();
    }

    quarter.to_string()
}

/// 從 MOPS 的分派情形中找出與這次除權息事件對應的那一筆。
///
/// 一家季配公司同一年會有多次除權息，也會有多筆分派情形，因此不能只用代號比對。
/// 這裡以「金額完全相符」作為配對條件：現金與股票股利同時吻合，且**只有一筆**吻合時
/// 才算配對成功。有兩筆以上同額的期別時無法分辨，一律視為配對失敗，交給第二階段。
pub(super) fn match_allotment<'a>(
    announcement: &ExDividendAnnouncement,
    allotments: &'a HashMap<String, Vec<DividendAllotment>>,
) -> Option<&'a DividendAllotment> {
    let candidates = allotments.get(&announcement.stock_symbol)?;
    let year = announcement.ex_date.year();

    // 股利所屬年度不是除權息當年（季配、半年配），就是前一年（年配隔年發放）。
    let mut matched = candidates.iter().filter(|allotment| {
        (allotment.year_of_dividend == year || allotment.year_of_dividend == year - 1)
            && amount_matches(announcement, allotment)
    });

    let first = matched.next()?;
    if matched.next().is_some() {
        return None;
    }

    Some(first)
}

/// 判斷公告金額與分派情形是否吻合。
///
/// 公告未提供金額（ETF 的「待公告」）時無從比對，一律視為不吻合，
/// 避免把未公布的事件硬塞進某個期別。
fn amount_matches(announcement: &ExDividendAnnouncement, allotment: &DividendAllotment) -> bool {
    let cash = if announcement.is_cash {
        match announcement.cash_dividend {
            Some(value) => value,
            None => return false,
        }
    } else {
        Decimal::ZERO
    };

    let stock = if announcement.is_stock {
        match announcement.stock_dividend() {
            Some(value) => value,
            None => return false,
        }
    } else {
        Decimal::ZERO
    };

    cash == allotment.cash_dividend() && stock == allotment.stock_dividend()
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::super::fixtures::{allotment, announcement, resolved};
    use super::super::source::index_allotments;
    use super::*;

    /// 同一年有兩期金額相同時無法分辨，必須放棄配對而不是隨便挑一筆。
    #[test]
    fn test_match_allotment_rejects_ambiguous_amounts() {
        let announcements = announcement("1102", (2026, 7, 1), true, false, Some(dec!(2.3)), None);
        let allotments = index_allotments(vec![
            allotment("1102", 2026, "Q1", dec!(2.3), Decimal::ZERO),
            allotment("1102", 2026, "Q2", dec!(2.3), Decimal::ZERO),
        ]);

        assert!(match_allotment(&announcements, &allotments).is_none());
    }

    /// 金額比對必須以數值為準，`5.000000` 與 `5` 是同一個金額。
    #[test]
    fn test_amount_matches_ignores_scale() {
        let ann = announcement(
            "2330",
            (2026, 9, 17),
            true,
            false,
            Some(dec!(5.000000)),
            None,
        );
        let allot = allotment("2330", 2026, "Q1", dec!(5), Decimal::ZERO);

        assert!(amount_matches(&ann, &allot));
    }

    /// 未公告金額（ETF 的「待公告」）不可配對，否則會綁到錯誤的期別。
    #[test]
    fn test_amount_matches_rejects_unannounced_cash() {
        let ann = announcement("00929", (2026, 9, 17), true, false, None, None);
        let allot = allotment("00929", 2026, "Q1", dec!(0.1), Decimal::ZERO);

        assert!(!amount_matches(&ann, &allot));
    }

    /// 配股率要換算成元之後再比對：0.1 股/股 等於 1 元。
    #[test]
    fn test_amount_matches_converts_stock_ratio() {
        let ann = announcement("1231", (2026, 8, 20), false, true, None, Some(dec!(0.1)));
        let allot = allotment("1231", 2025, "", Decimal::ZERO, dec!(1.0));

        assert!(amount_matches(&ann, &allot));
    }

    /// 跨年發放：12 月除息、隔年 1 月發放，資料列要掛在發放年度底下。
    #[test]
    fn test_resolve_paid_year_prefers_payable_date_for_periodic_dividend() {
        let ann = announcement("2882", (2025, 12, 20), true, false, Some(dec!(3.0)), None);

        // 季配且有發放日 → 用發放日的年份。
        assert_eq!(
            resolve_paid_year(
                &ann,
                Some(&resolved("Q4", dec!(3.0), None)),
                Some("2026-01-15")
            ),
            2026
        );
        // 年配則維持除權息日的年份。
        assert_eq!(
            resolve_paid_year(
                &ann,
                Some(&resolved("", dec!(3.0), None)),
                Some("2026-01-15")
            ),
            2025
        );
        // 來源已判定發放年度時一律優先。
        assert_eq!(
            resolve_paid_year(&ann, Some(&resolved("Q4", dec!(3.0), Some(2026))), None),
            2026
        );
        // 什麼都沒有時退回除權息日的年份。
        assert_eq!(resolve_paid_year(&ann, None, None), 2025);
    }
}
