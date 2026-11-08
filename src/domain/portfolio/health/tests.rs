//! 持股健檢的基本面警示的測試。

use super::*;

/// 四種警示各自的門檻：營收年減滿 3 個月、三率都下降、營業利益率為負、近四季自由現金流為負。
#[test]
fn warnings_follow_each_threshold() {
    let all = FundamentalSignals {
        revenue_decline_months: Some(3),
        operating_margin: Some(-1.5),
        margin_changes: [Some(-2.0), Some(-1.0), Some(-0.5)],
        free_cash_flow_ttm: Some(-1),
    };
    assert_eq!(all.margin_trend(), Some(MarginTrend::AllDown));
    assert_eq!(
        all.warnings(),
        [
            HoldingWarning::RevenueDecline,
            HoldingWarning::MarginsAllDown,
            HoldingWarning::OperatingLoss,
            HoldingWarning::NegativeFreeCashFlow
        ]
    );
    assert_eq!(
        HoldingWarning::NegativeFreeCashFlow.label(),
        "自由現金流為負"
    );

    let healthy = FundamentalSignals {
        revenue_decline_months: Some(2),
        operating_margin: Some(0.0),
        margin_changes: [Some(1.0), Some(-1.0), Some(0.5)],
        free_cash_flow_ttm: Some(0),
    };
    assert_eq!(healthy.margin_trend(), Some(MarginTrend::Mixed));
    assert!(healthy.warnings().is_empty());

    let up = FundamentalSignals {
        margin_changes: [Some(1.0), Some(2.0), Some(0.1)],
        ..Default::default()
    };
    assert_eq!(up.margin_trend(), Some(MarginTrend::AllUp));
    assert!(up.warnings().is_empty());
}

/// 查不到的項目不發警示，任一率無法比較時不判斷三率方向。
#[test]
fn missing_signals_raise_no_warning() {
    let signals = FundamentalSignals {
        margin_changes: [Some(-2.0), None, Some(-0.5)],
        ..Default::default()
    };
    assert_eq!(signals.margin_trend(), None);
    assert!(signals.warnings().is_empty());
    assert!(FundamentalSignals::default().warnings().is_empty());
}
