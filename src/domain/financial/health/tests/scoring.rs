//! Piotroski F 分數各項指標與比率計算的測試。

use super::*;

fn passed(criteria: &[HealthCriterion]) -> Vec<(&'static str, Option<bool>)> {
    criteria.iter().map(|c| (c.key, c.passed)).collect()
}

#[test]
fn piotroski_scores_each_criterion_in_fixed_order() {
    let (current, previous) = healthy_pair();
    let criteria = piotroski(&current, &previous);
    assert_eq!(
        passed(&criteria),
        vec![
            ("roa_positive", Some(true)),
            ("operating_cash_flow_positive", Some(true)),
            ("roa_improved", Some(true)),
            ("cash_flow_exceeds_net_income", Some(true)),
            ("leverage_not_increased", Some(false)),
            ("current_ratio_improved", Some(true)),
            ("no_new_shares", Some(true)),
            ("gross_margin_improved", Some(true)),
            ("asset_turnover_improved", Some(true)),
        ]
    );
    assert_eq!(tally(&criteria), (8, 9));

    // 比率取 4 位小數、金額原樣；分類與單位照表。
    let roa = &criteria[0];
    assert_eq!((roa.value, roa.baseline), (Some(0.0088), Some(0.0)));
    assert_eq!((roa.category, roa.unit), ("profitability", "ratio"));
    let accrual = &criteria[3];
    assert_eq!(
        (accrual.value, accrual.baseline),
        (Some(900.0), Some(460.0))
    );
    assert_eq!(accrual.unit, "thousand_twd");
    let leverage = &criteria[4];
    assert_eq!(
        (leverage.value, leverage.baseline),
        (Some(0.2115), Some(0.2083))
    );
    assert_eq!(leverage.category, "leverage_liquidity");
    assert_eq!(criteria[8].category, "operating_efficiency");
}

/// 長期負債比與股本「持平」算得分；其餘「提高」類指標持平不得分。
#[test]
fn piotroski_treats_unchanged_leverage_and_shares_as_passing() {
    let (current, _) = healthy_pair();
    let criteria = piotroski(&current, &current);
    let by_key: Vec<_> = passed(&criteria);
    assert_eq!(by_key[2], ("roa_improved", Some(false)));
    assert_eq!(by_key[4], ("leverage_not_increased", Some(true)));
    assert_eq!(by_key[6], ("no_new_shares", Some(true)));
    assert_eq!(by_key[7], ("gross_margin_improved", Some(false)));
}

/// 金融業的流動／非流動負債都是 0：長期負債比與流動比率不適用，不算分也不算失分。
#[test]
fn piotroski_skips_liquidity_criteria_without_liability_breakdown() {
    let (mut current, mut previous) = healthy_pair();
    for figures in [&mut current, &mut previous] {
        figures.current_assets = Some(0);
        figures.current_liabilities = Some(0);
        figures.non_current_liabilities = Some(0);
    }
    let criteria = piotroski(&current, &previous);
    assert_eq!(criteria[4].passed, None);
    assert_eq!(criteria[4].value, None);
    assert_eq!(criteria[5].passed, None);
    assert_eq!(tally(&criteria), (7, 7));
}

/// 沒有前一年度：只剩不需比較的 3 項可判定。
#[test]
fn piotroski_without_previous_year_evaluates_three_criteria() {
    let (current, _) = healthy_pair();
    let criteria = piotroski(&current, &YearFigures::default());
    let evaluated: Vec<_> = criteria
        .iter()
        .filter(|c| c.passed.is_some())
        .map(|c| c.key)
        .collect();
    assert_eq!(
        evaluated,
        [
            "roa_positive",
            "operating_cash_flow_positive",
            "cash_flow_exceeds_net_income"
        ]
    );
    assert_eq!(
        tally(&piotroski(&YearFigures::default(), &YearFigures::default())),
        (0, 0)
    );
}

#[test]
fn ratio_requires_both_values_and_non_zero_denominator() {
    assert_eq!(ratio(Some(1), Some(4)), Some(0.25));
    assert_eq!(ratio(Some(1), Some(0)), None);
    assert_eq!(ratio(None, Some(4)), None);
    assert_eq!(ratio(Some(1), None), None);
    assert_eq!(round_ratio(Some(0.123_456)), Some(0.1235));
}
