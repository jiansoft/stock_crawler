//! `/portfolio/rebalance` 與 `/portfolio/targets` 的 request/response DTO 與 OpenAPI schema。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// 查詢持股再平衡建議的 Query 參數。
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct PortfolioRebalanceQueryParams {
    /// 會員編號；未帶或 0 為所有部位（預設 0）。
    #[param(default = 0, minimum = 0)]
    pub member_id: Option<i64>,
    /// 再平衡維度：`stock`（個股）、`industry`（產業）、`etf_look_through`（ETF穿透）。預設為 `stock`。
    #[param(default = "stock")]
    pub dimension: Option<String>,
    /// 交易單位股數：`1000`（整張）或 `1`（零股）。預設為 `1000`。
    #[param(default = 1000)]
    pub lot_size: Option<u32>,
    /// 容忍帶百分比（例如 2.0 代表偏離在 ±2.0% 內不建議交易），預設 2.0。
    #[param(default = 2.0, minimum = 0.0)]
    pub tolerance_percent: Option<f64>,
    /// 目前帳戶可用現金（元），預設 0.0。
    #[param(default = 0.0, minimum = 0.0)]
    pub available_cash: Option<f64>,
    /// 券商手續費折讓（例如 0.28 代表 28 折，1.0 為無折讓），預設 1.0。
    #[param(default = 1.0, minimum = 0.0, maximum = 1.0)]
    pub fee_discount: Option<f64>,
}

/// 查詢目標配置設定的 Query 參數。
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct PortfolioTargetsQueryParams {
    /// 會員編號；未帶或 0 為所有部位（預設 0）。
    #[param(default = 0, minimum = 0)]
    pub member_id: Option<i64>,
    /// 再平衡維度：`stock`、`industry`、`etf_look_through`。若未指定則回傳所有維度。
    pub dimension: Option<String>,
}

/// 目標配置項目。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct PortfolioTargetItemDto {
    /// 配置標的鍵值：股票代號（個股或穿透維度）或產業名稱（產業維度）。
    pub target_key: String,
    /// 目標配置權重百分比（0.0 ~ 100.0）。
    pub target_percent: f64,
}

/// 設定目標配置權重的 Request Body。
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PutPortfolioTargetsRequest {
    /// 會員編號；未帶或 0 為所有部位（預設 0）。
    #[schema(default = 0, minimum = 0)]
    pub member_id: Option<i64>,
    /// 再平衡維度：`stock`、`industry`、`etf_look_through`。
    pub dimension: String,
    /// 目標配置比例清單（各項比例之和不得超過 100%）。
    pub targets: Vec<PortfolioTargetItemDto>,
}

/// 目標配置設定清單回應。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct PortfolioTargetsResponse {
    /// 會員編號。
    pub member_id: i64,
    /// 再平衡維度（若查詢全部則為 all）。
    pub dimension: String,
    /// 目前設定之目標配置清單。
    pub targets: Vec<PortfolioTargetItemDto>,
    /// 目標現金或未指定比例（100% − 各標的目標權重總和）。
    pub target_cash_percent: f64,
    /// 免責聲明文字。
    pub disclaimer: &'static str,
}

/// 個股維度再平衡建議項目。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct StockRebalanceItemDto {
    /// 股票代號。
    pub symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 現有持有股數。
    pub current_shares: i64,
    /// 目前每股市價。
    pub current_price: f64,
    /// 現有市值（元）。
    pub current_value: f64,
    /// 現有權重百分比（占總投資組合市值）。
    pub current_weight: f64,
    /// 目標權重百分比。
    pub target_weight: f64,
    /// 偏離百分點（目標 − 目前；正值代表偏低需加碼，負值代表偏高需減碼）。
    pub deviation_percent: f64,
    /// 建議動作：`buy`、`sell` 或 `hold`。
    pub action: String,
    /// 建議調整股數（正數，若為 hold 則為 0）。
    pub suggested_shares: i64,
    /// 預估交易金額（股數 × 目前市價）。
    pub suggested_amount: f64,
    /// 預估交易成本（手續費 ＋ 證交稅，手續費含低消 20 元）。
    pub estimated_cost: f64,
    /// 預估交易淨額（買入為 金額 ＋ 費用；賣出為 金額 − 費用）。
    pub net_amount: f64,
}

/// 產業維度再平衡建議項目。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct IndustryRebalanceItemDto {
    /// 產業名稱。
    pub industry: String,
    /// 現有市值（元）。
    pub current_value: f64,
    /// 現有權重百分比。
    pub current_weight: f64,
    /// 目標權重百分比。
    pub target_weight: f64,
    /// 偏離百分點。
    pub deviation_percent: f64,
    /// 現有持股中屬於該產業的標的清單（作為建議加減碼之候選）。
    pub candidate_stocks: Vec<String>,
}

/// ETF 穿透維度再平衡建議項目。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct EtfLookThroughRebalanceItemDto {
    /// 成分股股票代號。
    pub stock_symbol: String,
    /// 成分股股票名稱。
    pub name: String,
    /// 直接持有市值（元）。
    pub direct_value: f64,
    /// 透過 ETF 間接持有市值（元）。
    pub indirect_value: f64,
    /// 穿透後合計總市值（元）。
    pub total_value: f64,
    /// 穿透後現有權重百分比。
    pub current_weight: f64,
    /// 目標權重百分比。
    pub target_weight: f64,
    /// 偏離百分點。
    pub deviation_percent: f64,
}

/// 現金流試算摘要。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct RebalanceCashSummaryDto {
    /// 期初可用現金（元）。
    pub initial_cash: f64,
    /// 預估賣出實收淨額合計（元）。
    pub estimated_sell_net: f64,
    /// 預估買入需支付總額合計（元）。
    pub estimated_buy_total: f64,
    /// 可用資金總額（期初現金 ＋ 賣出實收淨額）。
    pub available_capital: f64,
    /// 需新增資金（若買入總額大於可用資金，則為不足之金額；否則為 0）。
    pub required_additional_cash: f64,
    /// 交易後預估剩餘現金（元）。
    pub remaining_cash: f64,
}

/// 投資組合持股再平衡建議完整回應。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct PortfolioRebalanceResponse {
    /// 會員編號。
    pub member_id: i64,
    /// 試算維度。
    pub dimension: String,
    /// 投資組合股票總市值（元）。
    pub total_equity_value: f64,
    /// 投資組合總資產市值（股票市值 ＋ 可用現金）。
    pub total_portfolio_value: f64,
    /// 目標現金或未指定比例（100% − 各標的目標權重總和）。
    pub target_cash_percent: f64,
    /// 採用之容忍帶（%）。
    pub tolerance_percent: f64,
    /// 採用之交易單位股數（1 或 1000）。
    pub lot_size: u32,
    /// 個股維度建議項目（僅在 dimension 為 stock 時有值）。
    pub stock_items: Vec<StockRebalanceItemDto>,
    /// 產業維度建議項目（僅在 dimension 為 industry 時有值）。
    pub industry_items: Vec<IndustryRebalanceItemDto>,
    /// ETF 穿透維度建議項目（僅在 dimension 為 etf_look_through 時有值）。
    pub etf_items: Vec<EtfLookThroughRebalanceItemDto>,
    /// 現金流試算摘要。
    pub cash_summary: RebalanceCashSummaryDto,
    /// 免責聲明文字。
    pub disclaimer: &'static str,
}
