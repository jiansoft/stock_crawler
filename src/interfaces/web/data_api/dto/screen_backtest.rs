//! `/stocks/screen/backtest` 的 request、response 與 OpenAPI schema。
//!
//! 只用每天都有留存的資料（相對強度、法人連續買超、技術訊號）選股，避免用到當時還沒公布的
//! 財報；報酬來自 `daily_forward_return`（基準日收盤到之後第 N 個交易日收盤的還原報酬，等同含息）。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{MarketParamValue, SignalIndicatorValue, SignalKindValue};

/// 條件選股回測的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct ScreenBacktestParams {
    /// 回測起日（`YYYY-MM-DD`）；省略為迄日往前 1,095 天（約三年）。
    pub(crate) from: Option<String>,
    /// 回測迄日（`YYYY-MM-DD`）；省略為最新有相對強度的交易日。起訖最長 3,653 天（約十年）。
    pub(crate) to: Option<String>,
    /// 持有天數（交易日）：5、20（預設）、60、120 或 240；每隔這麼多個交易日換股一次，
    /// 持有到下一次換股，報酬為這段期間的還原報酬。
    #[param(default = 20)]
    pub(crate) hold_days: Option<u16>,
    /// 每期最多持有幾檔（1–100，預設 20）：符合條件的股票依相對強度分數由高到低取前 N 檔，等權持有。
    #[param(minimum = 1, maximum = 100, default = 20)]
    pub(crate) limit: Option<u8>,
    /// 市場：`all`（預設）、`twse` 或 `tpex`；大盤基準也依此市場計算。
    #[param(value_type = MarketParamValue, default = "all")]
    pub(crate) market: Option<String>,
    /// 只選這個產業（產業名稱，例如「半導體業」）的股票；大盤基準不受影響。
    pub(crate) industry: Option<String>,
    /// 換股當天相對強度分數至少這麼高（1–99）。
    #[param(minimum = 1, maximum = 99)]
    pub(crate) min_rs_score: Option<u8>,
    /// 換股當天相對強度分數至多這麼高（1–99）。
    #[param(minimum = 1, maximum = 99)]
    pub(crate) max_rs_score: Option<u8>,
    /// 換股當天外資（不含外資自營商）已連續買超至少這麼多天（1–20），算法同
    /// `/market/institutional-flow`；籌碼資料從 2019 年起。
    #[param(minimum = 1, maximum = 20)]
    pub(crate) min_foreign_streak: Option<u8>,
    /// 換股當天投信已連續買超至少這麼多天（1–20）；規則同 `min_foreign_streak`。
    #[param(minimum = 1, maximum = 20)]
    pub(crate) min_trust_streak: Option<u8>,
    /// 只選換股前 `hold_days` 個交易日內（含換股當天）出現過這個來源訊號的股票。
    #[param(value_type = Option<SignalIndicatorValue>)]
    pub(crate) indicator: Option<String>,
    /// 只選換股前 `hold_days` 個交易日內出現過這種訊號的股票；可與 `indicator` 併用。
    #[param(value_type = Option<SignalKindValue>)]
    pub(crate) kind: Option<String>,
}

/// 一次換股（一期）的結果。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestPeriod {
    /// 換股日（收盤買進）。
    pub(crate) date: String,
    /// 這期持有的檔數；0 代表沒有符合條件的股票，這期空手（報酬 0）。
    pub(crate) holdings: i64,
    /// 持股等權平均報酬（%）；期間內下市等沒有報酬的股票不列入平均，空手為 `null`。
    pub(crate) avg_return: Option<f64>,
    /// 同一天同市場全部母體股票的平均報酬（%）。
    pub(crate) benchmark_return: Option<f64>,
    /// 依相對強度分數由高到低的前 10 檔代號。
    pub(crate) symbols: Vec<String>,
}

/// 整段回測的彙總；沒有任何一期時數值為 `null`。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestSummary {
    /// 期數。
    pub(crate) periods: i64,
    /// 有持股的期數。
    pub(crate) invested_periods: i64,
    /// 有持股的期平均持有檔數。
    pub(crate) avg_holdings: Option<f64>,
    /// 累積報酬（%）：各期報酬連乘，空手的期數報酬為 0。
    pub(crate) total_return: Option<f64>,
    /// 大盤基準的累積報酬（%）。
    pub(crate) benchmark_total_return: Option<f64>,
    /// 年化報酬（%），以每年約 245 個交易日換算。
    pub(crate) annualized_return: Option<f64>,
    /// 大盤基準的年化報酬（%）。
    pub(crate) benchmark_annualized_return: Option<f64>,
    /// 最大回撤（%，負值）：以每期期末的累積淨值計算，看不到期中的波動。
    pub(crate) max_drawdown: Option<f64>,
    /// 大盤基準的最大回撤（%）。
    pub(crate) benchmark_max_drawdown: Option<f64>,
    /// 報酬大於 0 的期數比例（0–1）。
    pub(crate) win_rate: Option<f64>,
    /// 報酬高於大盤基準的期數比例（0–1）。
    pub(crate) beat_market_rate: Option<f64>,
    /// 每期平均報酬（%，空手算 0）。
    pub(crate) avg_period_return: Option<f64>,
    /// 大盤基準的每期平均報酬（%）。
    pub(crate) avg_benchmark_return: Option<f64>,
}

/// 條件選股回測結果。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct ScreenBacktestResponse {
    /// 回測起日。
    pub(crate) from: String,
    /// 回測迄日。
    pub(crate) to: String,
    /// 市場：`all`、`twse` 或 `tpex`。
    pub(crate) market: String,
    /// 持有天數（交易日）。
    pub(crate) hold_days: u16,
    /// 每期最多持有檔數。
    pub(crate) limit: u8,
    /// 篩選的產業；未篩選為 `null`。
    pub(crate) industry: Option<String>,
    /// 相對強度分數下限；未篩選為 `null`。
    pub(crate) min_rs_score: Option<u8>,
    /// 相對強度分數上限；未篩選為 `null`。
    pub(crate) max_rs_score: Option<u8>,
    /// 外資連續買超天數下限；未篩選為 `null`。
    pub(crate) min_foreign_streak: Option<u8>,
    /// 投信連續買超天數下限；未篩選為 `null`。
    pub(crate) min_trust_streak: Option<u8>,
    /// 訊號來源；未篩選為 `null`。
    pub(crate) indicator: Option<String>,
    /// 訊號種類；未篩選為 `null`。
    pub(crate) kind: Option<String>,
    /// 報酬是否還原權值；固定為 `true`。
    pub(crate) adjusted: bool,
    /// 整段回測的彙總。
    pub(crate) summary: BacktestSummary,
    /// 各期結果，由舊到新；只列已滿持有天數的期數。
    pub(crate) periods: Vec<BacktestPeriod>,
}
