//! 期別判定：資料模型與第一階段配對的測試。

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
