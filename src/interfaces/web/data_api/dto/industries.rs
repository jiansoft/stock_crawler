//! `/market/industries` 的 request、response 與 OpenAPI schema。
//!
//! 各產業（上市櫃普通股，不含 ETF）在指定交易日、或往前 N 個交易日區間的漲跌表現、
//! 成交值占比與估值中位數。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::MarketParamValue;

/// 產業表現 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndustryPerformanceParams {
    /// 交易日，格式 `YYYY-MM-DD`；未提供時取最新交易日，遇休市日取之前最近的交易日。
    pub(crate) date: Option<String>,
    /// 市場：`all`（預設）、`twse` 或 `tpex`。
    #[param(value_type = MarketParamValue, default = "all")]
    pub(crate) market: Option<String>,
    /// 區間交易日數，預設 1（當日漲跌幅），範圍 1–60；大於 1 時以 `base_date` 收盤價計算區間漲跌（未還原除權息）。
    #[param(minimum = 1, maximum = 60, default = 1)]
    pub(crate) days: Option<u8>,
}

/// 產業內漲幅或跌幅最大的個股。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct IndustryMover {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 漲跌幅（%）。
    pub(crate) change_percent: f64,
}

/// 單一產業的表現。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct IndustryPerformance {
    /// 產業分類 id。
    pub(crate) industry_id: i32,
    /// 產業名稱；未分類時為空字串。
    pub(crate) industry_name: String,
    /// 當日有成交的檔數。
    pub(crate) stocks: usize,
    /// 上漲檔數。
    pub(crate) advancers: usize,
    /// 下跌檔數。
    pub(crate) decliners: usize,
    /// 平盤檔數。
    pub(crate) unchanged: usize,
    /// 個股漲跌幅中位數（%）；沒有可比較的個股時為 `null`。
    pub(crate) median_change_percent: Option<f64>,
    /// 市值加權漲跌幅（%，以期初市值加權）；沒有發行股數資料時為 `null`。
    pub(crate) cap_weighted_change_percent: Option<f64>,
    /// 當日成交金額合計（元）。
    pub(crate) trade_value: f64,
    /// 成交金額占所選市場的比例（%）。
    pub(crate) trade_value_share_percent: Option<f64>,
    /// 本益比中位數（只算 > 0 的值）。
    pub(crate) median_price_earning_ratio: Option<f64>,
    /// 股價淨值比中位數（只算 > 0 的值）。
    pub(crate) median_price_to_book_ratio: Option<f64>,
    /// 漲幅最大的個股；沒有上漲的個股時為 `null`。
    pub(crate) top_gainer: Option<IndustryMover>,
    /// 跌幅最大的個股；沒有下跌的個股時為 `null`。
    pub(crate) top_loser: Option<IndustryMover>,
}

/// 產業表現回應，`industries` 依市值加權漲跌幅由高到低排序。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct IndustryPerformanceResponse {
    /// 資料所屬交易日。
    pub(crate) data_as_of: String,
    /// 區間起點交易日（`days` 個交易日前）；沒有更早的資料時為 `null`。
    pub(crate) base_date: Option<String>,
    /// 市場：`all`、`twse` 或 `tpex`。
    pub(crate) market: String,
    /// 區間交易日數。
    pub(crate) days: u8,
    /// 各產業表現。
    pub(crate) industries: Vec<IndustryPerformance>,
}
