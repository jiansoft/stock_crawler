//! 市場估值水位（`/market/valuation`）的 request 與 response。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::MarketParamValue;

/// 市場估值水位的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct MarketValuationParams {
    /// 迄日（`YYYY-MM-DD`）；省略為今天，取這天（含）之前的資料。
    pub(crate) date: Option<String>,
    /// 回看幾年，預設 5，範圍 1–10。
    #[param(minimum = 1, maximum = 10, default = 5)]
    pub(crate) years: Option<u8>,
    /// 市場：`all`（預設，上市加上櫃）、`twse` 或 `tpex`。
    #[param(value_type = MarketParamValue, default = "all")]
    pub(crate) market: Option<String>,
}

/// 單一交易日的市場本益比與股價淨值比中位數。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct MarketValuationPoint {
    /// 交易日（`YYYY-MM-DD`）。
    pub(crate) date: String,
    /// 本益比為正（不含虧損股與 ETF）的上市櫃股票本益比中位數；當天沒有時為 `null`。
    pub(crate) median_pe: Option<f64>,
    /// 股價淨值比為正的上市櫃股票淨值比中位數；當天沒有時為 `null`。
    pub(crate) median_pb: Option<f64>,
    /// 計入本益比中位數的股票數。
    pub(crate) pe_samples: i64,
    /// 計入股價淨值比中位數的股票數。
    pub(crate) pb_samples: i64,
}

/// 期間內每日中位數的分布，以及最新一天在其中的位置。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct ValuationRatioStats {
    /// 最新一天的中位數。
    pub(crate) current: f64,
    /// 期間內中位數不高於最新一天的交易日比例（%，0–100）；越高代表估值越接近期間高點。
    pub(crate) percentile: f64,
    /// 期間最低。
    pub(crate) min: f64,
    /// 第 25 百分位。
    pub(crate) p25: f64,
    /// 中位數。
    pub(crate) median: f64,
    /// 第 75 百分位。
    pub(crate) p75: f64,
    /// 期間最高。
    pub(crate) max: f64,
    /// 參與計算的交易日數。
    pub(crate) days: usize,
}

/// 市場估值水位的成功回應。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct MarketValuationResponse {
    /// 最新一個有估值資料的交易日（`YYYY-MM-DD`）。
    pub(crate) data_as_of: String,
    /// 期間第一個有估值資料的交易日（`YYYY-MM-DD`）。
    pub(crate) from: String,
    /// 回看年數。
    pub(crate) years: u8,
    /// 市場條件。
    pub(crate) market: String,
    /// 最新一天的中位數與樣本數。
    pub(crate) current: MarketValuationPoint,
    /// 本益比中位數的分布；期間沒有本益比資料時為 `null`。
    pub(crate) pe: Option<ValuationRatioStats>,
    /// 股價淨值比中位數的分布；期間沒有淨值比資料時為 `null`。
    pub(crate) pb: Option<ValuationRatioStats>,
    /// 每月最後一個交易日的中位數，由舊到新。
    pub(crate) monthly: Vec<MarketValuationPoint>,
}
