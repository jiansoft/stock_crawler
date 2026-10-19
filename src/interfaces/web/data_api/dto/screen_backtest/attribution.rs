//! 相對類 0050 基準的報酬歸因（`/stocks/screen/backtest/attribution`）的 response。

use serde::Serialize;
use utoipa::ToSchema;

/// 報酬歸因中一個產業（或現金）的效果；效果為各期相加（百分點，不複利、未扣交易成本）。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestIndustryEffect {
    /// 產業名稱；策略部位的現金（大盤濾網空手、單檔上限分不完）為「現金」，查無產業為「未分類」。
    pub(crate) industry: String,
    /// 策略部位在這個產業的平均權重（%，各期平均，沒持有的期數算 0）。
    pub(crate) avg_weight: f64,
    /// 基準在這個產業的平均權重（%）。
    pub(crate) benchmark_avg_weight: f64,
    /// 配置效果：多配或少配這個產業，乘上該產業基準報酬相對基準整體的差。
    pub(crate) allocation: f64,
    /// 選股效果：以基準權重計，策略在這個產業選到的股票相對基準同產業股票的報酬差。
    pub(crate) selection: f64,
    /// 交互效果：多配或少配的部分乘上選股的報酬差。
    pub(crate) interaction: f64,
    /// 三項合計：這個產業對策略相對基準超額報酬的貢獻。
    pub(crate) total: f64,
}

/// 一段期間的報酬歸因合計（各期相加，百分點，不複利、未扣交易成本）。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestAttributionTotals {
    /// 有基準資料、納入歸因的期數。
    pub(crate) periods: i64,
    /// 策略部位各期報酬相加（%）。
    pub(crate) strategy_return: f64,
    /// 基準各期報酬相加（%）。
    pub(crate) benchmark_return: f64,
    /// 0050 同期各期報酬相加（%），用來看基準與 0050 的差距；全部缺資料為 `null`。
    pub(crate) etf_return: Option<f64>,
    /// 超額報酬：策略減基準（= 配置 + 選股 + 交互）。
    pub(crate) excess_return: f64,
    /// 配置效果合計。
    pub(crate) allocation: f64,
    /// 選股效果合計。
    pub(crate) selection: f64,
    /// 交互效果合計。
    pub(crate) interaction: f64,
}

/// 一年的報酬歸因（年度依換股日）。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestAttributionYear {
    /// 年度。
    pub(crate) year: i32,
    /// 這一年的合計。
    pub(crate) totals: BacktestAttributionTotals,
    /// 這一年影響最大的 5 個產業（依三項合計的絕對值由大到小）。
    pub(crate) industries: Vec<BacktestIndustryEffect>,
}

/// 條件選股回測的報酬歸因（Brinson-Fachler）：把策略部位相對類 0050 基準的超額報酬，拆成產業配置、
/// 選股與交互三項。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct ScreenBacktestAttributionResponse {
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
    /// 換股日偏移（交易日）。
    pub(crate) rebalance_offset: u16,
    /// 基準的檔數：每個換股日同市場母體中近 20 個交易日平均成交金額最高的 N 檔，依平均成交金額加權
    /// （近似 0050 的市值加權）。
    pub(crate) benchmark_size: i64,
    /// 整段的合計。
    pub(crate) summary: BacktestAttributionTotals,
    /// 依年度的合計與影響最大的產業，由舊到新。
    pub(crate) years: Vec<BacktestAttributionYear>,
    /// 整段各產業（含現金）的效果，依三項合計由大到小。
    pub(crate) industries: Vec<BacktestIndustryEffect>,
}
