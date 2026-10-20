//! `/portfolio/summary` 的 request、response 與 OpenAPI schema。
//!
//! 目前持有（未賣出）部位的總覽；計算規則見 [`crate::domain::portfolio::summary`]。
//! 金額單位為新台幣元，百分比為 0–100。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::domain::financial::entity::MarginTrend;
use crate::domain::portfolio::health::HoldingWarning;
use crate::domain::portfolio::summary::{
    HoldingSummary, IndustrySummary, PortfolioSummary, WeightedMetric,
};

/// OpenAPI 文件使用的持股基本面警示。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema。
pub(crate) enum PortfolioWarningValue {
    /// 最新月營收起連續 3 個月以上年減。
    RevenueDecline,
    /// 最近一季毛利率、營業利益率、稅後淨利率都比去年同季低（三率三降）。
    MarginsAllDown,
    /// 最近一季營業利益為負。
    OperatingLoss,
    /// 截至最近一季的近四季自由現金流合計為負。
    NegativeFreeCashFlow,
}

/// OpenAPI 文件使用的三率方向。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema。
pub(crate) enum PortfolioMarginTrendValue {
    /// 三率都比去年同季高（三率三升）。
    AllUp,
    /// 三率都比去年同季低（三率三降）。
    AllDown,
    /// 有升有降或持平。
    Mixed,
}

/// 警示的 API 契約值。
pub(crate) fn warning_code(warning: HoldingWarning) -> &'static str {
    match warning {
        HoldingWarning::RevenueDecline => "revenue_decline",
        HoldingWarning::MarginsAllDown => "margins_all_down",
        HoldingWarning::OperatingLoss => "operating_loss",
        HoldingWarning::NegativeFreeCashFlow => "negative_free_cash_flow",
    }
}

/// 三率方向的 API 契約值。
pub(crate) fn margin_trend_code(trend: MarginTrend) -> &'static str {
    match trend {
        MarginTrend::AllUp => "all_up",
        MarginTrend::AllDown => "all_down",
        MarginTrend::Mixed => "mixed",
    }
}

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
    /// 最新一筆日線的官方本益比（證交所／櫃買中心）；虧損或沒有資料時為 `null`。
    pub(crate) pe_ratio: Option<f64>,
    /// 最新一筆日線的官方股價淨值比；沒有資料時為 `null`。
    pub(crate) pb_ratio: Option<f64>,
    /// 相對強度分數 1–99（同 `/market/relative-strength`，資料日見 `rs_as_of`）；
    /// ETF、上市未滿 240 個交易日或暫停交易時為 `null`。
    pub(crate) rs_score: Option<u8>,
    /// 截至 `chip_as_of` 的外資連續買賣超天數：正數為連續買超、負數為連續賣超、0 為當天沒有買賣；
    /// 當天沒有籌碼資料時為 `null`。
    pub(crate) foreign_streak: Option<i16>,
    /// 截至 `chip_as_of` 的投信連續買賣超天數，規則同 `foreign_streak`。
    pub(crate) trust_streak: Option<i16>,
    /// 最新月營收月份（`YYYY-MM`）；沒有或超過 2 個月沒更新時為 `null`，以下兩個營收欄位同。
    pub(crate) revenue_month: Option<String>,
    /// 最新月營收年增率（%）。
    pub(crate) revenue_yoy_percent: Option<f64>,
    /// 最新月營收起往前連續年減的月數（最多看 12 個月；0 代表最新一個月沒有年減）。
    pub(crate) revenue_decline_months: Option<u8>,
    /// 最近一季（Yahoo 單季財報）期間，如 `2026-Q2`；沒有、落後超過 2 季或屬於金融保險業（營收口徑
    /// 不同，三率沒有可比性）時為 `null`，以下四個三率欄位同。
    pub(crate) margin_period: Option<String>,
    /// 最近一季毛利率（%）。
    pub(crate) gross_margin_percent: Option<f64>,
    /// 最近一季營業利益率（%）。
    pub(crate) operating_margin_percent: Option<f64>,
    /// 最近一季稅後淨利率（%）。
    pub(crate) net_margin_percent: Option<f64>,
    /// 三率與去年同季相比的方向；去年同季查不到時為 `null`。
    #[schema(value_type = Option<PortfolioMarginTrendValue>)]
    pub(crate) margins_trend: Option<String>,
    /// 截至最近一季的近四季自由現金流合計（新台幣千元）；四季不齊、落後超過 2 季或屬於金融保險業時為
    /// `null`。
    pub(crate) free_cash_flow_ttm: Option<i64>,
    /// 基本面警示：營收連續 3 個月以上年減、三率三降、營業虧損、近四季自由現金流為負；沒有時為空陣列。
    #[schema(value_type = Vec<PortfolioWarningValue>)]
    pub(crate) warnings: Vec<String>,
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
            pe_ratio: position.pe_ratio.map(decimal_to_f64),
            pb_ratio: position.pb_ratio.map(decimal_to_f64),
            rs_score: None,
            foreign_streak: None,
            trust_streak: None,
            revenue_month: None,
            revenue_yoy_percent: None,
            revenue_decline_months: None,
            margin_period: None,
            gross_margin_percent: None,
            operating_margin_percent: None,
            net_margin_percent: None,
            margins_trend: None,
            free_cash_flow_ttm: None,
            warnings: Vec::new(),
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
    /// 各持股 `rs_score` 的資料日；查無相對強度資料時為 `null`。
    pub(crate) rs_as_of: Option<String>,
    /// 各持股 `foreign_streak`、`trust_streak` 的資料日（每晚 21:50 入庫）；查無籌碼資料時為 `null`。
    pub(crate) chip_as_of: Option<String>,
    /// 有基本面警示（`holdings[].warnings` 不為空）的檔數。
    pub(crate) warning_symbols: usize,
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
    /// 組合本益比：市值加權的調和平均（Σ市值 ÷ Σ(市值 ÷ 本益比)，等同總市值 ÷ 依持股比例分得的盈餘），
    /// 虧損的部位不列入；沒有任何部位有本益比時為 `null`。
    pub(crate) weighted_pe: Option<WeightedMetricItem>,
    /// 組合股價淨值比，算法同 `weighted_pe`；沒有任何部位有淨值比時為 `null`。
    pub(crate) weighted_pb: Option<WeightedMetricItem>,
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
            rs_as_of: None,
            chip_as_of: None,
            warning_symbols: 0,
            symbols: summary.symbols,
            market_value: summary.market_value,
            cost: summary.cost,
            unrealized_gain: summary.unrealized_gain,
            unrealized_gain_percent: summary.unrealized_gain_percent,
            dividends_received: summary.dividends_received,
            weighted_dividend_yield: summary.weighted_dividend_yield.map(Into::into),
            weighted_f_score: summary.weighted_f_score.map(Into::into),
            weighted_pe: summary.weighted_pe.map(Into::into),
            weighted_pb: summary.weighted_pb.map(Into::into),
            industries: summary.industries.into_iter().map(Into::into).collect(),
            holdings: summary.holdings.into_iter().map(Into::into).collect(),
        }
    }
}

fn decimal_to_f64(value: rust_decimal::Decimal) -> f64 {
    use rust_decimal::prelude::ToPrimitive;
    value.round_dp(4).to_f64().unwrap_or_default()
}
