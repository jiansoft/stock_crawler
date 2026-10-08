use super::*;

/// 建立一筆兩種推估法都算得出來的樣本。
///
/// 累計 1,000,000 千元＝10 億元；股數 1 億股 ⇒ 每股累計營收 10 元。
/// 錨定法：錨點 EPS 3 元、錨點營收 600,000 千元 ⇒ 3 × (1,000,000 ÷ 600,000) ＝ 5 元。
/// 淨利率法：10 元 × 40% ＝ 4 元。兩者刻意給不同答案，才能分辨用了哪一條路徑。
fn revenue_alert(date: i64) -> HoldingRevenueAlert {
    HoldingRevenueAlert {
        stock_symbol: "2330".to_string(),
        stock_name: "台積電".to_string(),
        industry_name: Some("半導體業".to_string()),
        monthly: dec!(514805337),
        monthly_accumulated: dec!(1000000),
        compared_with_last_month: dec!(10.09),
        compared_with_last_year_same_month: dec!(53.32),
        accumulated_compared_with_last_year: dec!(41.2),
        issued_share: 100_000_000,
        net_income_margin: Some(dec!(40)),
        anchor_eps: Some(dec!(3)),
        anchor_accumulated_revenue: Some(dec!(600000)),
        anchor_quarter_no: Some(2),
        ttm_eps: None,
        ttm_revenue: None,
        reg_slope: None,
        reg_intercept: None,
        reg_r2: None,
        reg_quarters: None,
        date,
    }
}

#[test]
fn accumulated_months_reads_the_month_part_of_yyyymm() {
    assert_eq!(revenue_alert(202601).accumulated_months(), 1);
    assert_eq!(revenue_alert(202612).accumulated_months(), 12);
}

// 有錨點就用錨點，不會退回淨利率法（4 元）。
#[test]
fn estimate_eps_prefers_reported_quarters() {
    let estimate = revenue_alert(202608).estimate_eps().expect("estimate");

    assert_eq!(estimate.basis, EpsEstimateBasis::ReportedQuarters);
    assert_eq!(estimate.accumulated, dec!(5));
    // 8 個月做出 5 元，線性外推全年 7.5 元。
    assert_eq!(estimate.annual, dec!(7.5));
}

// 今年還沒公布季報（1~3 月）時退回淨利率法。
#[test]
fn estimate_eps_falls_back_to_net_income_margin() {
    let mut alert = revenue_alert(202603);
    alert.anchor_eps = None;
    alert.anchor_accumulated_revenue = None;

    let estimate = alert.estimate_eps().expect("estimate");

    assert_eq!(estimate.basis, EpsEstimateBasis::NetIncomeMargin);
    assert_eq!(estimate.accumulated, dec!(4));
    assert_eq!(estimate.annual, dec!(16));
}

// 錨點營收為 0 會讓比率除以零，必須當成沒有錨點而退回淨利率法。
#[test]
fn estimate_eps_falls_back_when_anchor_revenue_is_zero() {
    let mut alert = revenue_alert(202608);
    alert.anchor_accumulated_revenue = Some(Decimal::ZERO);

    let estimate = alert.estimate_eps().expect("estimate");

    assert_eq!(estimate.basis, EpsEstimateBasis::NetIncomeMargin);
    assert_eq!(estimate.accumulated, dec!(4));
}

// 錨點季底就是本期時比率為 1，推估值等於實際公布的累計 EPS。
#[test]
fn estimate_eps_equals_reported_eps_at_quarter_end() {
    let mut alert = revenue_alert(202606);
    alert.monthly_accumulated = dec!(600000);

    let estimate = alert.estimate_eps().expect("estimate");

    assert_eq!(estimate.accumulated, dec!(3));
}

// 12 月的累計就是全年，年化後不應該再被放大。
#[test]
fn estimate_eps_annual_equals_accumulated_in_december() {
    let estimate = revenue_alert(202612).estimate_eps().expect("estimate");

    assert_eq!(estimate.annual, estimate.accumulated);
}

// 虧損公司的錨點 EPS 是負的；近四季也不賺、營收步調正常時照比例外推，推估值跟著是負的。
#[test]
fn estimate_eps_keeps_negative_anchor() {
    let mut alert = revenue_alert(202608);
    alert.anchor_eps = Some(dec!(-1.2));
    // 新增兩個月共 200,000 千元，月均與錨點期間相同（步調 1 倍）。
    alert.monthly_accumulated = dec!(800000);

    let estimate = alert.estimate_eps().expect("estimate");

    assert_eq!(estimate.basis, EpsEstimateBasis::ReportedQuarters);
    assert_eq!(estimate.accumulated, dec!(-1.6));
    assert!(!estimate.volatile);
}

// 怡華 2026-08 的實際數字：上半年虧 1.07 元，7、8 月建案入帳營收暴增。
// 按比例外推會得到 -7.83（營收越多虧越多）；改用近四季淨利率後轉為獲利。
#[test]
fn estimate_eps_uses_trailing_margin_when_anchor_is_a_loss() {
    let mut alert = revenue_alert(202608);
    alert.anchor_eps = Some(dec!(-1.07));
    alert.anchor_accumulated_revenue = Some(dec!(367504));
    alert.monthly_accumulated = dec!(2690860);
    alert.ttm_eps = Some(dec!(2.9));
    alert.ttm_revenue = Some(dec!(736300));

    let estimate = alert.estimate_eps().expect("estimate");

    assert_eq!(
        estimate.basis,
        EpsEstimateBasis::ReportedQuartersWithTrailingMargin
    );
    assert_eq!(estimate.accumulated.round_dp(2), dec!(8.08));
    assert!(estimate.volatile, "新增月份月均營收是錨點期間的 19 倍");
}

// 錨點虧損、近四季也不賺、營收又劇變：沒有可信的淨利率，不推估也不退回淨利率法。
#[test]
fn estimate_eps_is_none_when_loss_anchor_meets_volatile_revenue() {
    let mut alert = revenue_alert(202608);
    alert.anchor_eps = Some(dec!(-1.07));
    alert.monthly_accumulated = dec!(3000000);
    alert.ttm_eps = Some(dec!(-0.5));
    alert.ttm_revenue = Some(dec!(900000));

    assert_eq!(alert.estimate_eps(), None);
}

/// 全坤建 2026-08 的實際數字：上半年虧 0.52 元、近四季也虧，7、8 月建案入帳。
/// 近 12 季（含 2024 年交屋）迴歸：每季固定 -0.284 元、每千元營收 9.606e-7 元，R² 0.77。
fn construction_alert() -> HoldingRevenueAlert {
    let mut alert = revenue_alert(202608);
    alert.anchor_eps = Some(dec!(-0.52));
    alert.anchor_accumulated_revenue = Some(dec!(126622));
    alert.monthly_accumulated = dec!(1327379);
    alert.ttm_eps = Some(dec!(-0.74));
    alert.ttm_revenue = Some(dec!(258378));
    alert.reg_slope = Some(dec!(0.0000009606));
    alert.reg_intercept = Some(dec!(-0.284));
    alert.reg_r2 = Some(dec!(0.77));
    alert.reg_quarters = Some(12);
    alert
}

// -0.52 ＋ 兩個月固定損益 (-0.284 × 2 ÷ 3) ＋ 新增營收 1,200,757 × 9.606e-7 ≈ 0.44。
#[test]
fn estimate_eps_uses_regression_when_loss_anchor_has_reliable_history() {
    let estimate = construction_alert().estimate_eps().expect("estimate");

    assert_eq!(
        estimate.basis,
        EpsEstimateBasis::ReportedQuartersWithRegression
    );
    assert_eq!(estimate.accumulated.round_dp(2), dec!(0.44));
    assert!(estimate.volatile);
}

// 迴歸可靠時優先於近四季淨利率。
#[test]
fn estimate_eps_prefers_regression_over_trailing_margin() {
    let mut alert = construction_alert();
    alert.ttm_eps = Some(dec!(2.9));

    let estimate = alert.estimate_eps().expect("estimate");

    assert_eq!(
        estimate.basis,
        EpsEstimateBasis::ReportedQuartersWithRegression
    );
}

// R² 不足、斜率不為正或季數不足時迴歸都不採用；此例近四季也虧、營收劇變，因此不推估。
#[test]
fn estimate_eps_rejects_unreliable_regression() {
    let mut low_r2 = construction_alert();
    low_r2.reg_r2 = Some(dec!(0.49));
    assert_eq!(low_r2.estimate_eps(), None);

    let mut negative_slope = construction_alert();
    negative_slope.reg_slope = Some(dec!(-0.0000001));
    assert_eq!(negative_slope.estimate_eps(), None);

    let mut few_quarters = construction_alert();
    few_quarters.reg_quarters = Some(7);
    assert_eq!(few_quarters.estimate_eps(), None);
}

// 錨點獲利時即使營收劇變也維持按比例外推，只標記為僅供參考。
#[test]
fn estimate_eps_flags_volatile_revenue_for_profitable_anchor() {
    let mut alert = revenue_alert(202608);
    alert.monthly_accumulated = dec!(1200000);

    let estimate = alert.estimate_eps().expect("estimate");

    assert_eq!(estimate.basis, EpsEstimateBasis::ReportedQuarters);
    assert_eq!(estimate.accumulated, dec!(6));
    assert!(estimate.volatile);
}

// 步調 = 新增月份月均 ÷ 錨點期間月均；本期就是錨點季底時沒有新增月份。
#[test]
fn revenue_pace_compares_monthly_average_after_anchor() {
    // 錨點 Q2：600,000 ÷ 6 ＝ 100,000／月；新增兩個月 400,000 ÷ 2 ＝ 200,000／月。
    assert_eq!(revenue_alert(202608).revenue_pace(), Some(dec!(2)));

    let mut alert = revenue_alert(202606);
    alert.monthly_accumulated = dec!(600000);
    assert_eq!(alert.revenue_pace(), None);
}

// 兩種方法都缺料時不推估。
#[test]
fn estimate_eps_is_none_without_any_usable_input() {
    let mut alert = revenue_alert(202608);
    alert.anchor_eps = None;
    alert.anchor_accumulated_revenue = None;
    alert.net_income_margin = None;

    assert_eq!(alert.estimate_eps(), None);

    let mut alert = revenue_alert(202608);
    alert.anchor_eps = None;
    alert.anchor_accumulated_revenue = None;
    alert.issued_share = 0;

    assert_eq!(alert.estimate_eps(), None);
}

// 月份欄位異常（yyyyMM 的月份不在 1~12）時不推估，避免除以 0。
#[test]
fn estimate_eps_is_none_for_invalid_month() {
    assert_eq!(revenue_alert(202600).estimate_eps(), None);
}
