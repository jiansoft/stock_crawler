//! 持股健檢的基本面警示。
//!
//! 由最新月營收與最近一季財報（過期資料由查詢端排除）判斷三種警示：營收連續
//! [`REVENUE_DECLINE_WARNING_MONTHS`] 個月年減、三率（毛利率、營業利益率、稅後淨利率）都比去年
//! 同季低、最近一季營業虧損。

use crate::domain::financial::entity::MarginTrend;

/// 營收連續年減達這個月數就發警示。
pub const REVENUE_DECLINE_WARNING_MONTHS: u8 = 3;

/// 持股的基本面警示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldingWarning {
    /// 最新月營收起連續 [`REVENUE_DECLINE_WARNING_MONTHS`] 個月以上年減。
    RevenueDecline,
    /// 最近一季三率都比去年同季低（三率三降）。
    MarginsAllDown,
    /// 最近一季營業利益為負。
    OperatingLoss,
}

/// 一檔持股的基本面訊號；查不到或過期的項目為 `None`。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FundamentalSignals {
    /// 最新月營收起往前連續年減的月數（0 代表最新一個月沒有年減）。
    pub revenue_decline_months: Option<u8>,
    /// 最近一季營業利益率（%）。
    pub operating_margin: Option<f64>,
    /// 最近一季三率（毛利率、營業利益率、稅後淨利率）對去年同季的變化（百分點）。
    pub margin_changes: [Option<f64>; 3],
}

impl FundamentalSignals {
    /// 三率與去年同季相比的方向；任一率無法比較時為 `None`。
    pub fn margin_trend(&self) -> Option<MarginTrend> {
        MarginTrend::classify(self.margin_changes)
    }

    /// 依序列出觸發的警示：營收連續年減、三率三降、營業虧損。
    pub fn warnings(&self) -> Vec<HoldingWarning> {
        let mut warnings = Vec::new();
        if self
            .revenue_decline_months
            .is_some_and(|months| months >= REVENUE_DECLINE_WARNING_MONTHS)
        {
            warnings.push(HoldingWarning::RevenueDecline);
        }
        if self.margin_trend() == Some(MarginTrend::AllDown) {
            warnings.push(HoldingWarning::MarginsAllDown);
        }
        if self.operating_margin.is_some_and(|margin| margin < 0.0) {
            warnings.push(HoldingWarning::OperatingLoss);
        }
        warnings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 三種警示各自的門檻：營收年減滿 3 個月、三率都下降、營業利益率為負。
    #[test]
    fn warnings_follow_each_threshold() {
        let all = FundamentalSignals {
            revenue_decline_months: Some(3),
            operating_margin: Some(-1.5),
            margin_changes: [Some(-2.0), Some(-1.0), Some(-0.5)],
        };
        assert_eq!(all.margin_trend(), Some(MarginTrend::AllDown));
        assert_eq!(
            all.warnings(),
            [
                HoldingWarning::RevenueDecline,
                HoldingWarning::MarginsAllDown,
                HoldingWarning::OperatingLoss
            ]
        );

        let healthy = FundamentalSignals {
            revenue_decline_months: Some(2),
            operating_margin: Some(0.0),
            margin_changes: [Some(1.0), Some(-1.0), Some(0.5)],
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
}
