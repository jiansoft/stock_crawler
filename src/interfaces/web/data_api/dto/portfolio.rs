//! `/portfolio/summary` 的 request、response 與 OpenAPI schema。
//!
//! 目前持有（未賣出）部位的總覽；計算規則見 [`crate::domain::portfolio::summary`]。
//! 金額單位為新台幣元，百分比為 0–100。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::domain::portfolio::summary::{
    HoldingSummary, IndustrySummary, PortfolioSummary, WeightedMetric,
};

/// 持股總覽 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct PortfolioSummaryParams {
    /// 會員編號；0（預設）為全部會員合計。
    #[param(minimum = 0, default = 0)]
    pub(crate) member_id: Option<i64>,
}

/// 以市值加權的指標與覆蓋率。
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
pub(crate) struct WeightedMetricItem {
    /// 加權平均值。
    pub(crate) value: f64,
    /// 有此指標的部位占總市值的百分比。
    pub(crate) coverage_percent: f64,
}

impl From<WeightedMetric> for WeightedMetricItem {
    fn from(metric: WeightedMetric) -> Self {
        Self {
            value: metric.value,
            coverage_percent: metric.coverage_percent,
        }
    }
}

/// 單一產業的持股彙總。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct PortfolioIndustry {
    /// 產業分類編號；股票主檔找不到時為 `null`。
    pub(crate) industry_id: Option<i32>,
    /// 產業名稱。
    pub(crate) industry_name: String,
    /// 該產業持股的市值合計。
    pub(crate) market_value: f64,
    /// 占總市值的百分比。
    pub(crate) weight_percent: f64,
    /// 持有幾檔。
    pub(crate) symbols: usize,
}

impl From<IndustrySummary> for PortfolioIndustry {
    fn from(industry: IndustrySummary) -> Self {
        Self {
            industry_id: industry.industry_id,
            industry_name: industry.industry_name,
            market_value: industry.market_value,
            weight_percent: industry.weight_percent,
            symbols: industry.symbols,
        }
    }
}

/// 單一股票的持有部位（同代號多筆買進已合併）。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct PortfolioHolding {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 產業分類編號。
    pub(crate) industry_id: Option<i32>,
    /// 產業名稱。
    pub(crate) industry_name: String,
    /// 持有股數。
    pub(crate) shares: i64,
    /// 持有成本（配股取得的股數成本為 0）。
    pub(crate) cost: f64,
    /// 最新收盤價；沒有報價時為 `null`。
    pub(crate) closing_price: Option<f64>,
    /// 市值；沒有報價時為 `null`。
    pub(crate) market_value: Option<f64>,
    /// 占總市值的百分比；沒有報價時為 `null`。
    pub(crate) weight_percent: Option<f64>,
    /// 未實現損益；沒有報價時為 `null`。
    pub(crate) unrealized_gain: Option<f64>,
    /// 未實現報酬率；沒有報價或成本為 0 時為 `null`。
    pub(crate) unrealized_gain_percent: Option<f64>,
    /// 這些部位累計已領股利。
    pub(crate) dividends_received: f64,
    /// 最新且仍在 31 天內的殖利率；缺值或過期時為 `null`。
    pub(crate) dividend_yield_percent: Option<f64>,
    /// 評分年度落後不超過 2 年的 Piotroski F 分數；缺值或過期時為 `null`。
    pub(crate) f_score: Option<u8>,
}

impl From<HoldingSummary> for PortfolioHolding {
    fn from(holding: HoldingSummary) -> Self {
        let position = holding.position;
        Self {
            stock_symbol: position.stock_symbol,
            name: position.name,
            industry_id: position.industry_id,
            industry_name: position.industry_name,
            shares: position.shares,
            cost: decimal_to_f64(position.cost),
            closing_price: position.closing_price.map(decimal_to_f64),
            market_value: holding.market_value,
            weight_percent: holding.weight_percent,
            unrealized_gain: holding.unrealized_gain,
            unrealized_gain_percent: holding.unrealized_gain_percent,
            dividends_received: decimal_to_f64(position.dividends_received),
            dividend_yield_percent: position.dividend_yield_percent.map(decimal_to_f64),
            f_score: position.f_score.and_then(|score| u8::try_from(score).ok()),
        }
    }
}

/// 持股組合總覽。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct PortfolioSummaryResponse {
    /// 會員編號；0 為全部會員合計。
    pub(crate) member_id: i64,
    /// 各持股最新收盤日中最新的一天；沒有持股或報價時為 `null`。
    pub(crate) data_as_of: Option<String>,
    /// 持有檔數。
    pub(crate) symbols: usize,
    /// 總市值（只計有報價的部位）。
    pub(crate) market_value: f64,
    /// 總成本。
    pub(crate) cost: f64,
    /// 有報價部位的未實現損益合計。
    pub(crate) unrealized_gain: f64,
    /// 未實現報酬率；有報價部位的成本為 0 時為 `null`。
    pub(crate) unrealized_gain_percent: Option<f64>,
    /// 目前持股累計已領股利。
    pub(crate) dividends_received: f64,
    /// 市值加權殖利率；沒有任何部位有殖利率時為 `null`。
    pub(crate) weighted_dividend_yield: Option<WeightedMetricItem>,
    /// 市值加權 F 分數；沒有任何部位有 F 分數時為 `null`。
    pub(crate) weighted_f_score: Option<WeightedMetricItem>,
    /// 產業分布，依市值由大到小。
    pub(crate) industries: Vec<PortfolioIndustry>,
    /// 各持股，依市值由大到小。
    pub(crate) holdings: Vec<PortfolioHolding>,
}

impl PortfolioSummaryResponse {
    /// 由領域彙總組出回應。
    pub(crate) fn new(
        member_id: i64,
        data_as_of: Option<String>,
        summary: PortfolioSummary,
    ) -> Self {
        Self {
            member_id,
            data_as_of,
            symbols: summary.symbols,
            market_value: summary.market_value,
            cost: summary.cost,
            unrealized_gain: summary.unrealized_gain,
            unrealized_gain_percent: summary.unrealized_gain_percent,
            dividends_received: summary.dividends_received,
            weighted_dividend_yield: summary.weighted_dividend_yield.map(Into::into),
            weighted_f_score: summary.weighted_f_score.map(Into::into),
            industries: summary.industries.into_iter().map(Into::into).collect(),
            holdings: summary.holdings.into_iter().map(Into::into).collect(),
        }
    }
}

fn decimal_to_f64(value: rust_decimal::Decimal) -> f64 {
    use rust_decimal::prelude::ToPrimitive;
    value.round_dp(4).to_f64().unwrap_or_default()
}
