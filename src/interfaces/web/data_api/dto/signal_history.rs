//! `/market/signals/performance` 與 `/stocks/{symbol}/signals` 的 request、response 與 OpenAPI schema。
//!
//! 訊號來自收盤後預先計算的 `daily_technical_signal`，之後的表現來自 `daily_forward_return`
//! （基準日收盤到之後第 5／20／60／120／240 個交易日收盤的還原報酬，等同含息）。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{MarketParamValue, SignalIndicatorValue, SignalKindValue};

/// 訊號績效統計的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct SignalPerformanceParams {
    /// 統計起日（`YYYY-MM-DD`）；省略為迄日往前 364 天。
    pub(crate) from: Option<String>,
    /// 統計迄日（`YYYY-MM-DD`）；省略為最新有訊號的交易日。起訖最長 1,096 天。
    pub(crate) to: Option<String>,
    /// 市場：`all`（預設）、`twse` 或 `tpex`；大盤基準也依此市場計算。
    #[param(value_type = MarketParamValue, default = "all")]
    pub(crate) market: Option<String>,
    /// 只統計這種訊號；省略為全部。
    #[param(value_type = Option<SignalKindValue>)]
    pub(crate) kind: Option<String>,
    /// 只統計這個來源的訊號；省略為全部。
    #[param(value_type = Option<SignalIndicatorValue>)]
    pub(crate) indicator: Option<String>,
    /// 只統計這個產業（產業名稱，例如「半導體業」）的訊號；大盤基準不受影響。
    pub(crate) industry: Option<String>,
    /// 只統計訊號當天相對強度分數至少這麼高的訊號（1–99，同 `/market/relative-strength`）；
    /// 沒有分數的訊號（ETF、上市未滿 240 個交易日）不列入。大盤基準不受影響。
    #[param(minimum = 1, maximum = 99)]
    pub(crate) min_rs_score: Option<u8>,
    /// 只統計訊號當天相對強度分數至多這麼高的訊號（1–99）；規則同 `min_rs_score`。
    #[param(minimum = 1, maximum = 99)]
    pub(crate) max_rs_score: Option<u8>,
    /// 只統計訊號當天外資（不含外資自營商）已連續買超至少這麼多天的訊號（1–20）。連續天數與
    /// `/market/institutional-flow` 同一算法（只看有籌碼資料的日子，淨額 0 會中斷）；籌碼資料從
    /// 2022-10 起，之前的訊號不列入。大盤基準不受影響。
    #[param(minimum = 1, maximum = 20)]
    pub(crate) min_foreign_streak: Option<u8>,
    /// 只統計訊號當天投信已連續買超至少這麼多天的訊號（1–20）；規則同 `min_foreign_streak`，
    /// 兩者都設時需同時符合。
    #[param(minimum = 1, maximum = 20)]
    pub(crate) min_trust_streak: Option<u8>,
    /// `year`：除了整段期間，再依訊號日的年度拆分統計（每個項目的 `years`、`benchmark_by_year`），
    /// 用來看訊號在不同年份（多頭、空頭）是否穩定；省略為不拆分。
    #[param(value_type = Option<GroupByParamValue>)]
    pub(crate) group_by: Option<String>,
}

/// 依年度拆分統計的 `group_by` 值。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GroupByParamValue {
    /// 依年度拆分。
    Year,
}

/// 某一年度（事件或訊號日所在的年度）的表現統計。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct PerformanceYear {
    /// 年度（西元）。
    pub(crate) year: i32,
    /// 該年度的出現次數。
    pub(crate) occurrences: i64,
    /// 依天期由短到長的表現；年底的事件之後的報酬可能落在下一年，仍算在事件的年度。
    pub(crate) horizons: Vec<HorizonStat>,
}

/// 某一年度的大盤基準。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BenchmarkYear {
    /// 年度（西元）。
    pub(crate) year: i32,
    /// 該年度每天同市場全部股票的平均報酬，依天期由短到長。
    pub(crate) benchmark: Vec<BenchmarkStat>,
}

/// 某天期（N 個交易日後）的表現統計；樣本為 0 時數值為 `null`。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct HorizonStat {
    /// 天期（交易日數）：5、20、60、120 或 240。
    pub(crate) days: u32,
    /// 已滿天期、有報酬的樣本數。
    pub(crate) samples: i64,
    /// 平均報酬（%）。
    pub(crate) avg_return: Option<f64>,
    /// 報酬中位數（%）。
    pub(crate) median_return: Option<f64>,
    /// 報酬大於 0 的比例（0–1）。
    pub(crate) win_rate: Option<f64>,
    /// 平均超額報酬（%）：每筆報酬減去同一天同市場全部股票的平均報酬。
    pub(crate) avg_excess_return: Option<f64>,
    /// 報酬高於同一天同市場平均的比例（0–1）。
    pub(crate) beat_market_rate: Option<f64>,
}

/// 單一來源與種類的訊號績效。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct SignalPerformanceItem {
    /// 訊號來源。
    pub(crate) indicator: String,
    /// 訊號種類。
    pub(crate) kind: String,
    /// 期間內出現次數（同一檔股票不同日各算一次）。
    pub(crate) occurrences: i64,
    /// 依天期由短到長的表現。
    pub(crate) horizons: Vec<HorizonStat>,
    /// `group_by=year` 時各年度的表現，由舊到新（只列有出現的年度）；未拆分為 `null`。
    pub(crate) years: Option<Vec<PerformanceYear>>,
}

/// 大盤基準：期間內每天同市場全部股票的平均報酬。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct BenchmarkStat {
    /// 天期（交易日數）。
    pub(crate) days: u32,
    /// 樣本數（股票 × 交易日）。
    pub(crate) samples: i64,
    /// 平均報酬（%）。
    pub(crate) avg_return: Option<f64>,
    /// 報酬大於 0 的比例（0–1）。
    pub(crate) win_rate: Option<f64>,
}

/// 訊號績效統計結果。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct SignalPerformanceResponse {
    /// 統計起日。
    pub(crate) from: String,
    /// 統計迄日。
    pub(crate) to: String,
    /// 市場：`all`、`twse` 或 `tpex`。
    pub(crate) market: String,
    /// 篩選的訊號種類；未篩選為 `null`。
    pub(crate) kind: Option<String>,
    /// 篩選的訊號來源；未篩選為 `null`。
    pub(crate) indicator: Option<String>,
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
    /// 拆分方式：`year` 或未拆分為 `null`。
    pub(crate) group_by: Option<String>,
    /// 報酬是否還原權值；固定為 `true`。
    pub(crate) adjusted: bool,
    /// 大盤基準，依天期由短到長。
    pub(crate) benchmark: Vec<BenchmarkStat>,
    /// `group_by=year` 時各年度的大盤基準，由舊到新；未拆分為 `null`。
    pub(crate) benchmark_by_year: Option<Vec<BenchmarkYear>>,
    /// 各來源與種類的績效，依來源、種類的固定順序。
    pub(crate) items: Vec<SignalPerformanceItem>,
}

/// 個股訊號歷史的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct StockSignalsParams {
    /// 迄日（`YYYY-MM-DD`）；省略為今天。
    pub(crate) date: Option<String>,
    /// 往前幾個日曆天（含迄日），預設 180，範圍 1–730。
    #[param(minimum = 1, maximum = 730, default = 180)]
    pub(crate) days: Option<u16>,
    /// 只列出這種訊號；省略為全部。
    #[param(value_type = Option<SignalKindValue>)]
    pub(crate) kind: Option<String>,
    /// 只列出這個來源的訊號；省略為全部。
    #[param(value_type = Option<SignalIndicatorValue>)]
    pub(crate) indicator: Option<String>,
    /// 最多回傳幾筆，預設 50，範圍 1–200；由新到舊。
    #[param(minimum = 1, maximum = 200, default = 50)]
    pub(crate) limit: Option<u8>,
}

/// 個股某日出現的訊號與之後的表現。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct StockSignalItem {
    /// 交易日。
    pub(crate) date: String,
    /// 訊號來源。
    pub(crate) indicator: String,
    /// 訊號種類。
    pub(crate) kind: String,
    /// 中文說明。
    pub(crate) description: String,
    /// 當日收盤價（元）。
    pub(crate) closing_price: Option<f64>,
    /// 當日漲跌幅（%）。
    pub(crate) change_percent: Option<f64>,
    /// 之後 5 個交易日的還原報酬（%）；未滿為 `null`。
    pub(crate) return_5d: Option<f64>,
    /// 之後 20 個交易日的還原報酬（%）；未滿為 `null`。
    pub(crate) return_20d: Option<f64>,
    /// 之後 60 個交易日的還原報酬（%）；未滿為 `null`。
    pub(crate) return_60d: Option<f64>,
    /// 之後 120 個交易日的還原報酬（%）；未滿為 `null`。
    pub(crate) return_120d: Option<f64>,
    /// 之後 240 個交易日的還原報酬（%）；未滿為 `null`。
    pub(crate) return_240d: Option<f64>,
}

/// 個股訊號歷史。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct StockSignalsResponse {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 查詢起日。
    pub(crate) from: String,
    /// 查詢迄日。
    pub(crate) to: String,
    /// 訊號與報酬是否以還原權值的價格計算；固定為 `true`。
    pub(crate) adjusted: bool,
    /// 符合條件的訊號筆數（未套用 `limit`）。
    pub(crate) total: i64,
    /// 由新到舊的訊號。
    pub(crate) items: Vec<StockSignalItem>,
}
