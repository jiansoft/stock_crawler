//! `/stocks/screen/backtest` 的 request、response 與 OpenAPI schema。
//!
//! 只用每天都有留存的資料（相對強度、法人連續買超、技術訊號）選股，避免用到當時還沒公布的
//! 財報；報酬來自 `daily_forward_return`（基準日收盤到之後第 N 個交易日收盤的還原報酬，等同含息；
//! 期間內下市的股票算到最後一筆收盤）。

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
    /// 換股緩衝（`limit`–200，預設等於 `limit` 即不緩衝）：上一期持有的股票只要這期仍符合條件、
    /// 且排在前 `keep_rank` 名就續抱，空出的名額才依相對強度補滿，用來降低換手與交易成本。
    #[param(minimum = 1, maximum = 200)]
    pub(crate) keep_rank: Option<u8>,
    /// 大盤濾網：換股日加權指數收盤低於自己的 N 日均線（20、60、120 或 240 個交易日）時這期空手，
    /// 上一期持股全部賣出；省略為不濾。
    pub(crate) market_filter_ma: Option<u16>,
    /// 市場廣度濾網（1–99）：換股日同市場站上 60 日均線的股票比例低於 N% 時這期空手，上一期持股
    /// 全部賣出；可與 `market_filter_ma` 併用（兩者都通過才持股）。統計從 2019 年起；省略為不濾。
    #[param(minimum = 1, maximum = 99)]
    pub(crate) market_filter_breadth: Option<u8>,
    /// 續抱條件（1–99）：上一期持有的股票即使不再符合選股條件（例如法人連買中斷、訊號過期），只要
    /// 換股當天相對強度分數不低於此值就續抱；省略為不續抱（只看 `keep_rank`）。
    #[param(minimum = 1, maximum = 99)]
    pub(crate) hold_min_rs_score: Option<u8>,
    /// 同產業最多持有幾檔（1–100）：每期持股（含續抱）同一產業超過 N 檔時，排名較後的跳過、
    /// 改由其他產業依相對強度補滿，用來降低產業集中；省略為不限。
    #[param(minimum = 1, maximum = 100)]
    pub(crate) max_per_industry: Option<u8>,
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
    /// 換股當天外資（含外資自營商）已連續買超至少這麼多天（1–60），算法同
    /// `/market/institutional-flow`；籌碼資料從 2019 年起。
    #[param(minimum = 1, maximum = 60)]
    pub(crate) min_foreign_streak: Option<u8>,
    /// 換股當天投信已連續買超至少這麼多天（1–60）；規則同 `min_foreign_streak`。
    #[param(minimum = 1, maximum = 60)]
    pub(crate) min_trust_streak: Option<u8>,
    /// 只選換股前 `hold_days` 個交易日內（含換股當天）出現過這個來源訊號的股票。
    #[param(value_type = Option<SignalIndicatorValue>)]
    pub(crate) indicator: Option<String>,
    /// 只選換股前 `hold_days` 個交易日內出現過這種訊號的股票；可與 `indicator` 併用。
    #[param(value_type = Option<SignalKindValue>)]
    pub(crate) kind: Option<String>,
    /// 手續費折扣（0–1，預設 1 為不打折；例如 0.28 為 2.8 折）：扣成本後的報酬以手續費 0.1425%
    /// 乘上折扣、賣出另加證交稅 0.3% 計算。
    #[param(minimum = 0.0, maximum = 1.0, default = 1.0)]
    pub(crate) fee_discount: Option<f64>,
}

/// 一次換股（一期）的結果。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestPeriod {
    /// 換股日（收盤買進）。
    pub(crate) date: String,
    /// 這期持有的檔數；0 代表沒有符合條件的股票或大盤濾網沒過，這期空手（報酬 0）。
    pub(crate) holdings: i64,
    /// 換股日加權指數低於濾網均線或市場廣度低於門檻而空手。
    pub(crate) market_filtered: bool,
    /// 持股等權平均報酬（%），未扣交易成本；期間內下市的股票算到最後一筆收盤，之後完全沒有收盤的
    /// 股票不列入平均；空手為 `null`。
    pub(crate) avg_return: Option<f64>,
    /// 扣除這次換股的手續費與證交稅後的報酬（%）；空手但賣出上一期持股時為負的成本。
    pub(crate) return_after_costs: f64,
    /// 換手率（0–1）：這期新買進的檔數占持股的比例，第一期為 1、空手為 0。
    pub(crate) turnover: f64,
    /// 同一天同市場全部母體股票的平均報酬（%）。
    pub(crate) benchmark_return: Option<f64>,
    /// 同一天買進 0050 持有同樣天數的還原報酬（%）；缺資料為 `null`。
    pub(crate) etf_return: Option<f64>,
    /// 依相對強度分數由高到低的前 10 檔代號（換手率以全部持股計算）。
    pub(crate) symbols: Vec<String>,
}

/// 整段回測的彙總；沒有任何一期時數值為 `null`。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestSummary {
    /// 期數。
    pub(crate) periods: i64,
    /// 有持股的期數。
    pub(crate) invested_periods: i64,
    /// 因大盤濾網（均線或市場廣度）空手的期數。
    pub(crate) market_filtered_periods: i64,
    /// 有持股的期平均持有檔數。
    pub(crate) avg_holdings: Option<f64>,
    /// 有持股的期平均換手率（0–1）。
    pub(crate) avg_turnover: Option<f64>,
    /// 累積報酬（%）：各期報酬連乘，空手的期數報酬為 0；未扣交易成本。
    pub(crate) total_return: Option<f64>,
    /// 扣除交易成本後的累積報酬（%）。
    pub(crate) total_return_after_costs: Option<f64>,
    /// 大盤基準的累積報酬（%）。
    pub(crate) benchmark_total_return: Option<f64>,
    /// 年化報酬（%），以每年約 245 個交易日換算；未扣交易成本。
    pub(crate) annualized_return: Option<f64>,
    /// 扣除交易成本後的年化報酬（%）。
    pub(crate) annualized_return_after_costs: Option<f64>,
    /// 大盤基準的年化報酬（%）。
    pub(crate) benchmark_annualized_return: Option<f64>,
    /// 最大回撤（%，負值）：以每期期末的累積淨值計算，看不到期中的波動。
    pub(crate) max_drawdown: Option<f64>,
    /// 扣除交易成本後的最大回撤（%）。
    pub(crate) max_drawdown_after_costs: Option<f64>,
    /// 大盤基準的最大回撤（%）。
    pub(crate) benchmark_max_drawdown: Option<f64>,
    /// 報酬大於 0 的期數比例（0–1），未扣交易成本。
    pub(crate) win_rate: Option<f64>,
    /// 報酬高於大盤基準的期數比例（0–1）。
    pub(crate) beat_market_rate: Option<f64>,
    /// 每期平均報酬（%，空手算 0）。
    pub(crate) avg_period_return: Option<f64>,
    /// 大盤基準的每期平均報酬（%）。
    pub(crate) avg_benchmark_return: Option<f64>,
    /// 每期都買 0050 的累積報酬（%）：實際買得到的大盤基準，未扣交易成本；全期都缺資料為 `null`。
    pub(crate) etf_total_return: Option<f64>,
    /// 0050 的年化報酬（%）。
    pub(crate) etf_annualized_return: Option<f64>,
    /// 0050 的最大回撤（%）。
    pub(crate) etf_max_drawdown: Option<f64>,
    /// 報酬高於 0050 的期數比例（0–1）。
    pub(crate) beat_etf_rate: Option<f64>,
    /// 年化波動率（%）：每期報酬（未扣成本）的標準差 × √每年期數；不足 2 期為 `null`。
    pub(crate) volatility: Option<f64>,
    /// 夏普值：扣交易成本後每期平均報酬 ÷ 每期標準差 × √每年期數，無風險利率以 0 計；
    /// 不足 2 期或報酬沒有波動為 `null`。
    pub(crate) sharpe_ratio: Option<f64>,
    /// 大盤基準的年化波動率（%）。
    pub(crate) benchmark_volatility: Option<f64>,
    /// 大盤基準的夏普值（未扣成本）。
    pub(crate) benchmark_sharpe_ratio: Option<f64>,
    /// 0050 的年化波動率（%）；全期都缺資料為 `null`。
    pub(crate) etf_volatility: Option<f64>,
    /// 0050 的夏普值（未扣成本）。
    pub(crate) etf_sharpe_ratio: Option<f64>,
}

/// 依換股日所屬年度彙總的一年（跨年的那一期算在換股日的年度）。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BacktestYear {
    /// 年度。
    pub(crate) year: i32,
    /// 這一年換股的期數。
    pub(crate) periods: i64,
    /// 這一年各期報酬連乘的累積報酬（%），空手算 0；未扣交易成本。
    pub(crate) total_return: f64,
    /// 扣除交易成本後的累積報酬（%）。
    pub(crate) total_return_after_costs: f64,
    /// 大盤基準（同市場全部母體平均）的累積報酬（%）。
    pub(crate) benchmark_total_return: f64,
    /// 0050 的累積報酬（%）；這一年都缺資料為 `null`。
    pub(crate) etf_total_return: Option<f64>,
    /// 報酬大於 0 的期數比例（0–1）。
    pub(crate) win_rate: f64,
    /// 報酬高於大盤基準的期數比例（0–1）。
    pub(crate) beat_market_rate: f64,
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
    /// 續抱的排名緩衝；等於 `limit` 為不緩衝。
    pub(crate) keep_rank: u8,
    /// 大盤濾網的均線天數；未設定為 `null`。
    pub(crate) market_filter_ma: Option<u16>,
    /// 市場廣度濾網門檻（%）；未設定為 `null`。
    pub(crate) market_filter_breadth: Option<u8>,
    /// 續抱的相對強度門檻；未設定為 `null`。
    pub(crate) hold_min_rs_score: Option<u8>,
    /// 同產業最多持有檔數；未設定為 `null`。
    pub(crate) max_per_industry: Option<u8>,
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
    /// 計算交易成本用的手續費折扣（0–1）。
    pub(crate) fee_discount: f64,
    /// 報酬是否還原權值；固定為 `true`。
    pub(crate) adjusted: bool,
    /// 可投資的大盤基準代號（固定為 `0050`）。
    pub(crate) etf_benchmark: String,
    /// 整段回測的彙總。
    pub(crate) summary: BacktestSummary,
    /// 依換股日年度的彙總，由舊到新；用來看策略在多頭、空頭年份是否穩定。
    pub(crate) years: Vec<BacktestYear>,
    /// 各期結果，由舊到新；只列已滿持有天數的期數。
    pub(crate) periods: Vec<BacktestPeriod>,
}
