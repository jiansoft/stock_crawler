//! 策略優勢快取查詢的 HTTP 契約；報酬以百分比、勝率以 0–1 表示。

use serde::Serialize;
use utoipa::ToSchema;

/// 一段期間的換股日偏移分布。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct StrategyEdgeWindow {
    /// 實際起日。
    pub(crate) from: String,
    /// 實際迄日。
    pub(crate) to: String,
    /// 偏移樣本數。
    pub(crate) offsets: usize,
    /// 扣成本年化報酬中位數（%）；無樣本為 null。
    pub(crate) median_return: Option<f64>,
    /// 0050 年化報酬中位數（%）；無樣本為 null。
    pub(crate) etf_median_return: Option<f64>,
    /// 勝 0050 的偏移比例（0–1）；無樣本為 null。
    pub(crate) beat_rate: Option<f64>,
}

/// 依近期一段評估的判斷；不會自動修改策略通知設定。
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StrategyEdgeVerdict {
    /// 優勢仍在。
    Holds,
    /// 優勢轉弱。
    Weakened,
    /// 缺少數字，無法判斷。
    Unknown,
}

/// 一個策略最近一次的檢查結果。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct StrategyEdgeItem {
    /// 策略名稱。
    pub(crate) name: String,
    /// 檢查日；這不是即時重算結果。
    pub(crate) checked: String,
    /// 近 24 個月的分布。
    pub(crate) recent: StrategyEdgeWindow,
    /// 2019 起全期分布；該段計算失敗為 null。
    pub(crate) full: Option<StrategyEdgeWindow>,
    /// 近期一段的判斷。
    pub(crate) verdict: StrategyEdgeVerdict,
}

/// 每月第一個交易日更新的策略優勢快取。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct StrategyEdgeResponse {
    /// 各策略的最近檢查。
    pub(crate) strategies: Vec<StrategyEdgeItem>,
    /// 成交假設：次日開盤。
    pub(crate) execution_price: String,
    /// 單邊滑價（基點）。
    pub(crate) slippage_bps: u16,
    /// 投資參考聲明。
    pub(crate) disclaimer: String,
}
