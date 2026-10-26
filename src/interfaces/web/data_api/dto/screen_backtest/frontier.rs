//! 0050 核心部位比例前緣（`/stocks/screen/backtest/frontier`）的 response。

use serde::Serialize;
use utoipa::ToSchema;

use super::BacktestOffsetDistribution;

/// 0050 核心部位前緣的一個比例：各換股日偏移混合後的績效分布。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestFrontierPoint {
    /// 0050 核心部位（%）：0 為純策略、100 為只持有 0050。
    pub(crate) etf_weight: u8,
    /// 這個比例下各偏移績效的分布（欄位與偏移穩健度相同）。
    pub(crate) distribution: BacktestOffsetDistribution,
    /// 樣本內（換股日早於分割日）的分布；未指定分割日為 `null`。
    pub(crate) before_split: Option<BacktestOffsetDistribution>,
    /// 樣本外（換股日不早於分割日）的分布；未指定分割日為 `null`。
    pub(crate) after_split: Option<BacktestOffsetDistribution>,
}

/// 0050 核心部位前緣：同一組條件搭配 0%～100% 的 0050，各比例的績效分布。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct ScreenBacktestFrontierResponse {
    pub(crate) from: String,
    /// 實際迄日（不晚於指定迄日、最近一個有相對強度的交易日）。
    pub(crate) to: String,
    pub(crate) market: String,
    pub(crate) hold_days: u16,
    pub(crate) limit: u8,
    /// 候選股排序方式。
    pub(crate) rank_by: String,
    /// 策略部位的持股加權方式。
    pub(crate) weighting: String,
    /// 單檔權重上限（%）；未設定為 `null`。
    pub(crate) max_weight: Option<u8>,
    /// 停損（%）；未設定為 `null`。
    pub(crate) stop_loss: Option<u8>,
    /// 移動停利（%）；未設定為 `null`。
    pub(crate) trailing_stop: Option<u8>,
    /// 計算交易成本用的手續費折扣（0–1）。
    pub(crate) fee_discount: f64,
    /// 成交價模型：`close`（換股日收盤）或 `next_open`（次一交易日開盤）。
    pub(crate) execution_model: String,
    /// 每次買進、賣出各計的滑價（基點）。
    pub(crate) slippage_bps: u16,
    /// 可投資的大盤基準代號（固定為 `0050`）。
    pub(crate) etf_benchmark: String,
    /// 跑了幾個換股日偏移（取法與偏移穩健度相同）。
    pub(crate) offsets: i64,
    /// 扣成本後夏普值中位數最高的比例（%）；同值取較低的比例，全部沒有數值為 `null`。
    pub(crate) best_sharpe_weight: Option<u8>,
    /// 扣成本後年化報酬中位數最高的比例（%）；同值取較低的比例，全部沒有數值為 `null`。
    pub(crate) best_return_weight: Option<u8>,
    /// 樣本內外分割日；未指定為 `null`。
    pub(crate) split: Option<String>,
    /// 只看樣本內時夏普值中位數最高的比例（之後看它在樣本外的表現）；未指定分割日為 `null`。
    pub(crate) best_sharpe_weight_before_split: Option<u8>,
    /// 各比例的分布，依比例由小到大（0、10、…、100）。
    pub(crate) weights: Vec<BacktestFrontierPoint>,
}
