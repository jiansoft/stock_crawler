//! 換股日偏移穩健度（`/stocks/screen/backtest/offsets`）的 response。

use serde::Serialize;
use utoipa::ToSchema;

use super::BacktestSummary;

/// 一組數值的分布（四分位以線性內插，四捨五入到小數兩位）。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestStat {
    pub(crate) min: f64,
    pub(crate) p25: f64,
    pub(crate) median: f64,
    pub(crate) mean: f64,
    pub(crate) p75: f64,
    pub(crate) max: f64,
}

/// 一個換股日偏移的回測彙總。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestOffsetResult {
    /// 換股日偏移（起日之後先跳過的交易日數）。
    pub(crate) rebalance_offset: u16,
    /// 與 `/stocks/screen/backtest` 相同的整段彙總。
    pub(crate) summary: BacktestSummary,
}

/// 各偏移績效的分布；沒有任何偏移有該項數值時為 `null`。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestOffsetDistribution {
    /// 扣交易成本後的年化報酬（%）。
    pub(crate) annualized_return_after_costs: Option<BacktestStat>,
    /// 扣成本後的夏普值。
    pub(crate) sharpe_ratio: Option<BacktestStat>,
    /// 扣成本後的最大回撤（%，負值）。
    pub(crate) max_drawdown_after_costs: Option<BacktestStat>,
    /// 0050 同期（同樣的換股日進出）的年化報酬（%）。
    pub(crate) etf_annualized_return: Option<BacktestStat>,
    /// 0050 同期的夏普值。
    pub(crate) etf_sharpe_ratio: Option<BacktestStat>,
    /// 年化波動率（%，未扣成本）。
    pub(crate) volatility: Option<BacktestStat>,
    /// 0050 同期的年化波動率（%）。
    pub(crate) etf_volatility: Option<BacktestStat>,
    /// 0050 同期的最大回撤（%，負值）。
    pub(crate) etf_max_drawdown: Option<BacktestStat>,
    /// 扣成本後年化報酬高於同偏移 0050 的偏移比例（0–1）。
    pub(crate) beat_etf_return_rate: Option<f64>,
    /// 扣成本後夏普值高於同偏移 0050 的偏移比例（0–1）。
    pub(crate) beat_etf_sharpe_rate: Option<f64>,
    /// 扣成本後最大回撤比同偏移 0050 淺（數值較大）的偏移比例（0–1）。
    pub(crate) beat_etf_drawdown_rate: Option<f64>,
}

/// 某一年各偏移的表現分布（年度依換股日，各偏移該年各期的累積報酬，年度各自從 1 起算）；
/// 起訖所在的年度可能不滿一年，各偏移的期數也可能不同。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestOffsetYear {
    /// 年度。
    pub(crate) year: i32,
    /// 這一年有換股的偏移數。
    pub(crate) offsets: i64,
    /// 扣交易成本後該年累積報酬（%）的分布。
    pub(crate) return_after_costs: Option<BacktestStat>,
    /// 0050 同期（同樣的換股日進出）該年累積報酬（%）的分布；全部缺資料為 `null`。
    pub(crate) etf_return: Option<BacktestStat>,
    /// 該年扣成本後累積報酬高於同偏移 0050 的偏移比例（0–1）；0050 全部缺資料為 `null`。
    pub(crate) beat_etf_rate: Option<f64>,
}

/// 換股日偏移穩健度結果。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct ScreenBacktestOffsetsResponse {
    pub(crate) from: String,
    /// 實際迄日（不晚於指定迄日、最近一個有相對強度的交易日）。
    pub(crate) to: String,
    pub(crate) market: String,
    pub(crate) hold_days: u16,
    pub(crate) limit: u8,
    /// 候選股排序方式。
    pub(crate) rank_by: String,
    /// 0050 核心部位（%）；0 為純策略。
    pub(crate) etf_weight: u8,
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
    /// 可投資的大盤基準代號（固定為 `0050`）。
    pub(crate) etf_benchmark: String,
    /// 各偏移績效的分布。
    pub(crate) distribution: BacktestOffsetDistribution,
    /// 依年度的分布，由舊到新：看策略在哪些年份穩定勝過 0050、哪些年份輸。
    pub(crate) years: Vec<BacktestOffsetYear>,
    /// 樣本內外分割日；未指定為 `null`。
    pub(crate) split: Option<String>,
    /// 換股日早於分割日的期數（樣本內）的分布；未指定分割日為 `null`。
    pub(crate) before_split: Option<BacktestOffsetDistribution>,
    /// 換股日不早於分割日的期數（樣本外）的分布；未指定分割日為 `null`。
    pub(crate) after_split: Option<BacktestOffsetDistribution>,
    /// 每個偏移的彙總，依偏移由小到大。
    pub(crate) offsets: Vec<BacktestOffsetResult>,
}
