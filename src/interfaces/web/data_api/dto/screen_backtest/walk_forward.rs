//! 滾動前推最佳化（`/stocks/screen/backtest/walk-forward`）的 response。

use serde::Serialize;
use utoipa::ToSchema;

use super::{BacktestOffsetDistribution, BacktestSummary};

/// 滾動前推的一年：用前幾年挑出的參數與這一年的結果。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestWalkForwardYear {
    /// 訓練窗口內因持有期跨過測試年起點而排除的期數。
    pub(crate) excluded_training_periods: usize,
    /// 套用的年度（換股日所屬年度）。
    pub(crate) year: i32,
    /// 挑出的 0050 核心部位（%）。
    pub(crate) etf_weight: u8,
    /// 挑出的單檔權重上限（%）；不設上限為 `null`。
    pub(crate) max_weight: Option<u8>,
    /// 挑出的換股緩衝名次。
    pub(crate) keep_rank: u8,
    /// 挑參數時（訓練期間）這組參數的扣成本後夏普值。
    pub(crate) train_sharpe_ratio: f64,
    /// 這一年的期數。
    pub(crate) periods: i64,
    /// 這一年扣成本後的累積報酬（%）。
    pub(crate) return_after_costs: f64,
    /// 同一組條件、不調參數（請求的 `etf_weight`、`max_weight`、`keep_rank`）這一年扣成本後的累積報酬（%）。
    pub(crate) baseline_return_after_costs: f64,
    /// 0050 同期的累積報酬（%）；這一年都缺資料為 `null`。
    pub(crate) etf_return: Option<f64>,
}

/// 滾動前推最佳化（walk-forward）：每年只用之前的資料挑參數再套用到這一年，看「每年重新調參數」
/// 能不能在沒看過的年份勝過 0050，以及比固定參數好還是壞。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct ScreenBacktestWalkForwardResponse {
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
    /// 最大回撤的口徑：多組回測一律為 `period_end`（換股期末），日頻回撤只在單次回測提供。
    pub(crate) drawdown_basis: String,
    /// 可投資的大盤基準代號（固定為 `0050`）。
    pub(crate) etf_benchmark: String,
    /// 訓練年數。
    pub(crate) train_years: u8,
    /// 每年比較的參數組合數（`etf_weight` × `max_weight` × `keep_rank`）。
    pub(crate) candidates: i64,
    /// 下列 `years`、`summary`、`baseline` 用的換股日偏移（`rebalance_offset`，預設 0）。
    pub(crate) rebalance_offset: u16,
    /// 每年挑出的參數與結果，由舊到新；訓練期間沒有夏普值的年份不列。
    pub(crate) years: Vec<BacktestWalkForwardYear>,
    /// 串起各年樣本外期數的彙總（換參數當下的調整成本不另計）。
    pub(crate) summary: BacktestSummary,
    /// 同樣年份、不調參數的彙總。
    pub(crate) baseline: BacktestSummary,
    /// 各換股日偏移（取法與偏移穩健度相同）滾動前推結果的分布。
    pub(crate) offsets: BacktestOffsetDistribution,
    /// 各換股日偏移不調參數的分布。
    pub(crate) baseline_offsets: BacktestOffsetDistribution,
}
