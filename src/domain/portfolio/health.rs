//! 持股健檢的基本面警示。
//!
//! 由最新月營收與最近一季財報（過期資料由查詢端排除）判斷四種警示：營收連續
//! [`REVENUE_DECLINE_WARNING_MONTHS`] 個月年減、三率（毛利率、營業利益率、稅後淨利率）都比去年
//! 同季低、最近一季營業虧損、近四季自由現金流合計為負。

use anyhow::Result;
use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::{core::declare::Industry, domain::financial::entity::MarginTrend};

/// 不取三率與自由現金流的產業：金融保險業的營收口徑與一般公司不同，三率沒有可比性；放款與存款
/// 也算在營業現金流裡，自由現金流同樣不適用。
pub const MARGIN_EXCLUDED_INDUSTRY: Industry = Industry::FinanceInsurance;

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
    /// 截至最近一季的近四季自由現金流合計為負。
    NegativeFreeCashFlow,
}

impl HoldingWarning {
    /// 警示的中文名稱。
    pub fn label(self) -> &'static str {
        match self {
            Self::RevenueDecline => "營收連續年減",
            Self::MarginsAllDown => "三率三降",
            Self::OperatingLoss => "營業虧損",
            Self::NegativeFreeCashFlow => "自由現金流為負",
        }
    }
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
    /// 截至最近一季的近四季自由現金流合計（新台幣千元）；四季不齊時為 `None`。
    pub free_cash_flow_ttm: Option<i64>,
}

impl FundamentalSignals {
    /// 三率與去年同季相比的方向；任一率無法比較時為 `None`。
    pub fn margin_trend(&self) -> Option<MarginTrend> {
        MarginTrend::classify(self.margin_changes)
    }

    /// 依序列出觸發的警示：營收連續年減、三率三降、營業虧損、自由現金流為負。
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
        if self.free_cash_flow_ttm.is_some_and(|amount| amount < 0) {
            warnings.push(HoldingWarning::NegativeFreeCashFlow);
        }
        warnings
    }
}

/// 一檔持股的最新月營收與最近一季三率；查不到或過期的項目為 `None`。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HoldingFundamentals {
    /// 股票代號。
    pub stock_symbol: String,
    /// 最新月營收月份（`YYYYMM`）。
    pub revenue_month: Option<i64>,
    /// 最新月營收年增率（%）。
    pub revenue_yoy: Option<Decimal>,
    /// 最近一季（Yahoo 單季財報）的年度與季別。
    pub margin_period: Option<(i32, i16)>,
    /// 最近一季毛利率、營業利益率、稅後淨利率（%）。
    pub margins: [Option<f64>; 3],
    /// 判斷警示用的訊號。
    pub signals: FundamentalSignals,
}

/// 持股基本面的查詢介面。
#[async_trait]
pub trait HoldingFundamentalsQuery: Send + Sync {
    /// 依今天判斷新鮮度：月營收超過 2 個月沒更新、財報落後超過 2 季視為過期；
    /// [`MARGIN_EXCLUDED_INDUSTRY`] 不取三率與自由現金流。查不到的代號不回傳。
    async fn fetch_holding_fundamentals(
        &self,
        symbols: &[String],
    ) -> Result<Vec<HoldingFundamentals>>;
}

#[cfg(test)]
mod tests;
