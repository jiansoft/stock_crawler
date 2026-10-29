//! 大盤未來報酬（`/market/forward-returns`）的 request 與 response。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{BenchmarkStat, BenchmarkYear, MarketParamValue};

/// 大盤未來報酬的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct MarketForwardReturnsParams {
    /// 迄日（`YYYY-MM-DD`）；省略為今天，統計這天（含）之前的基準日。
    pub(crate) date: Option<String>,
    /// 回看幾年，預設 3，範圍 1–20。
    #[param(minimum = 1, maximum = 20, default = 3)]
    pub(crate) years: Option<u8>,
    /// 市場：`all`（預設，上市加上櫃）、`twse` 或 `tpex`。
    #[param(value_type = MarketParamValue, default = "all")]
    pub(crate) market: Option<String>,
}

/// 期間內每個基準日買進同市場全部股票（等權）、N 個交易日後的報酬統計。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct MarketForwardReturnsResponse {
    /// 統計起日（迄日往前 `years` 年）。
    pub(crate) from: String,
    /// 統計迄日。
    pub(crate) to: String,
    /// 回看年數。
    pub(crate) years: u8,
    /// 市場：`all`、`twse` 或 `tpex`。
    pub(crate) market: String,
    /// 期間內最後一個有 5 日報酬的基準日。
    pub(crate) data_as_of: String,
    /// 整段期間的平均報酬與上漲比例，依天期由短到長；與 `/market/signals/performance` 的 `benchmark`
    /// 是同一份資料。
    pub(crate) benchmark: Vec<BenchmarkStat>,
    /// 各年度（基準日所在年度），由舊到新；年底的基準日之後的報酬可能落在下一年，仍算在基準日的年度。
    pub(crate) by_year: Vec<BenchmarkYear>,
}
