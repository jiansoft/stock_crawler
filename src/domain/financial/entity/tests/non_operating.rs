//! 錨點獲利但業外偏高時，未公布月份改用業外回到常態的獲利率推估。

use rust_decimal_macros::dec;

use super::*;

/// 錨點獲利、業外偏高的樣本：營業利益率 10%、業外占營收 30%、過去 8 季中位數 5%。
/// 業外回到常態的獲利率比值 ＝ (10 ＋ 5) ÷ (10 ＋ 30) ＝ 0.375。
fn excess_non_operating_alert(date: i64) -> HoldingRevenueAlert {
    let mut alert = revenue_alert(date);
    alert.anchor_operating_margin = Some(dec!(10));
    alert.anchor_non_operating_margin = Some(dec!(30));
    alert.typical_non_operating_margin = Some(dec!(5));
    alert
}

// 已實現的 3 元照財報；新增 400,000 千元 × 每千元 0.000005 元 × 0.375 ＝ 0.75。
// 年估：營收年化成 1,500,000，超過錨點 900,000 × 0.000005 × 0.375 ＝ 1.6875，
// 而不是把含超額業外的累計 3.75 再乘 12 ÷ 8（5.625）。
#[test]
fn estimate_eps_excludes_excess_non_operating_income_from_unreported_months() {
    let estimate = excess_non_operating_alert(202608)
        .estimate_eps()
        .expect("estimate");

    assert_eq!(
        estimate.basis,
        EpsEstimateBasis::ReportedQuartersWithNormalNonOperating
    );
    assert_eq!(estimate.accumulated, dec!(3.75));
    assert_eq!(estimate.annual, dec!(4.6875));
}

// 豪展 2026-10 的實際數字：上半年 EPS 4.37（Q2 業外占營收 37%），按比例外推得 8.15（年估 9.78）。
#[test]
fn estimate_eps_matches_4735_after_excluding_excess_non_operating_income() {
    let mut alert = revenue_alert(202610);
    alert.anchor_eps = Some(dec!(4.37));
    alert.anchor_accumulated_revenue = Some(dec!(488053));
    alert.monthly_accumulated = dec!(910673);
    alert.anchor_operating_margin = Some(dec!(7.7678));
    alert.anchor_non_operating_margin = Some(dec!(28.5620));
    alert.typical_non_operating_margin = Some(dec!(6.4390));

    let estimate = alert.estimate_eps().expect("estimate");

    assert_eq!(estimate.accumulated.round_dp(2), dec!(5.85));
    assert_eq!(estimate.annual.round_dp(2), dec!(6.49));
}

// 本期就是錨點季底：累計等於實際 EPS，年估仍只把未公布的半年按常態業外外推。
#[test]
fn estimate_eps_excludes_excess_non_operating_income_at_quarter_end() {
    let mut alert = excess_non_operating_alert(202606);
    alert.monthly_accumulated = dec!(600000);

    let estimate = alert.estimate_eps().expect("estimate");

    assert_eq!(estimate.accumulated, dec!(3));
    assert_eq!(estimate.annual, dec!(4.125));
}

// 本業虧損、獲利全靠業外：比值 (−5 ＋ 2) ÷ 15 ＝ −0.2，以 0 為下限，
// 未公布月份不加不減，累計與年估都停在已實現的 3 元，不會推出比已實現還低的 EPS。
#[test]
fn estimate_eps_floors_ratio_at_zero_when_profit_is_all_non_operating() {
    let mut alert = excess_non_operating_alert(202608);
    alert.anchor_operating_margin = Some(dec!(-5));
    alert.anchor_non_operating_margin = Some(dec!(20));
    alert.typical_non_operating_margin = Some(dec!(2));

    assert_eq!(alert.normal_non_operating_ratio(), Some(Decimal::ZERO));
    let estimate = alert.estimate_eps().expect("estimate");

    assert_eq!(
        estimate.basis,
        EpsEstimateBasis::ReportedQuartersWithNormalNonOperating
    );
    assert_eq!(estimate.accumulated, dec!(3));
    assert_eq!(estimate.annual, dec!(3));
}

// 不調整的情況：業外不高於中位數、打折不到一成、稅前虧損、資料不齊、錨點虧損——都維持原本的推估法。
#[test]
fn estimate_eps_keeps_proportional_when_non_operating_income_is_normal() {
    let cases = [
        // 業外低於中位數（例如一次性減損）：只往下修，不拉高。
        (Some(dec!(10)), Some(dec!(2)), Some(dec!(5))),
        // (30 ＋ 4) ÷ (30 ＋ 6) ≈ 0.944，差距不到一成。
        (Some(dec!(30)), Some(dec!(6)), Some(dec!(4))),
        // 稅前虧損。
        (Some(dec!(-40)), Some(dec!(10)), Some(dec!(1))),
        // 歷史不足 4 季。
        (Some(dec!(10)), Some(dec!(30)), None),
    ];
    for (operating, non_operating, typical) in cases {
        let mut alert = revenue_alert(202608);
        alert.anchor_operating_margin = operating;
        alert.anchor_non_operating_margin = non_operating;
        alert.typical_non_operating_margin = typical;

        let estimate = alert.estimate_eps().expect("estimate");

        assert_eq!(
            estimate.basis,
            EpsEstimateBasis::ReportedQuarters,
            "{operating:?} {non_operating:?} {typical:?}"
        );
        assert_eq!(estimate.accumulated, dec!(5));
        assert_eq!(estimate.annual, dec!(7.5));
    }

    // 錨點虧損時業外打折不適用，仍走虧損錨點的規則。
    let mut loss = excess_non_operating_alert(202608);
    loss.anchor_eps = Some(dec!(-1.2));
    loss.monthly_accumulated = dec!(800000);
    let estimate = loss.estimate_eps().expect("estimate");
    assert_eq!(estimate.basis, EpsEstimateBasis::ReportedQuarters);
    assert_eq!(estimate.accumulated, dec!(-1.6));
}
