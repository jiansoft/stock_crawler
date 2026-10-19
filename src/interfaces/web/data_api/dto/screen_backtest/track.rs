//! 具名策略實盤追蹤（`/strategy-presets/{name}/track`）的 response。

use serde::Serialize;
use utoipa::ToSchema;

/// 一期持股的一檔股票。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct StrategyTrackHolding {
    /// 股票代號。
    pub(crate) symbol: String,
    /// 股票名稱；查無為 `null`。
    pub(crate) name: Option<String>,
    /// 換股當天在策略部位的權重（%）。
    pub(crate) weight: f64,
    /// 換股日收盤到這期結束（下次換股日或最新交易日）收盤的還原報酬（%）；查不到日線為 `null`。
    pub(crate) return_percent: Option<f64>,
}

/// 一次換股（一期）的實際表現。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct StrategyTrackPeriod {
    /// 換股日（收盤買進）。
    pub(crate) date: String,
    /// 這期結束日：下次換股日，最後一期為最新交易日。
    pub(crate) end: String,
    /// 最後一期、還在持有中。
    pub(crate) ongoing: bool,
    /// 持有天數（交易日）。
    pub(crate) hold_days: u16,
    /// 0050 核心部位（%）。
    pub(crate) etf_weight: u8,
    /// 策略部位放現金的比例（%）。
    pub(crate) cash: f64,
    /// 大盤濾網沒過、策略部位空手。
    pub(crate) market_filtered: bool,
    /// 換股當天通知的持股。
    pub(crate) holdings: Vec<StrategyTrackHolding>,
    /// 策略部位的報酬（%，依權重、現金報酬 0、未扣成本）；換股當天或查不到日線為 `null`。
    pub(crate) strategy_return: Option<f64>,
    /// 0050 同期的還原報酬（%）。
    pub(crate) etf_return: Option<f64>,
    /// 組合報酬（%）：策略部位與 0050 核心部位依比例混合。
    pub(crate) portfolio_return: Option<f64>,
}

/// 具名策略的實盤追蹤：依換股通知當天記錄的持股，逐期計算實際表現並與 0050 比較。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct StrategyTrackResponse {
    /// 策略名稱。
    pub(crate) preset: String,
    /// 第一次記錄的換股日；還沒有紀錄為 `null`。
    pub(crate) from: Option<String>,
    /// 計算到的最新交易日；還沒有紀錄為 `null`。
    pub(crate) to: Option<String>,
    /// 各期組合報酬連乘的累積報酬（%，未扣成本）；有任一期查不到為 `null`。
    pub(crate) cumulative_return: Option<f64>,
    /// 同一段期間（第一次換股日到最新交易日）持有 0050 的還原報酬（%）。
    pub(crate) etf_cumulative_return: Option<f64>,
    /// 累積報酬減 0050（百分點）。
    pub(crate) excess_return: Option<f64>,
    /// 各期，由舊到新。
    pub(crate) periods: Vec<StrategyTrackPeriod>,
}
