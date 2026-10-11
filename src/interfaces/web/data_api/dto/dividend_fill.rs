//! `/stocks/{symbol}/dividend-fill` 的 request、response 與 OpenAPI schema。
//!
//! 每次除權息後，股價第幾個交易日回到除權息前一日收盤價（填息／填權）；
//! 規則見 [`crate::domain::dividend::fill`]。價格單位為新台幣元。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// 填息分析 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct DividendFillParams {
    /// 最近幾次除權息，預設 12，範圍 1–40。
    #[param(minimum = 1, maximum = 40, default = 12)]
    pub(crate) limit: Option<u8>,
}

/// 單次除權息的填息結果。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct DividendFillEvent {
    /// 除權息日，格式 `YYYY-MM-DD`；同一天的現金與股票股利合併成一次。
    pub(crate) ex_date: String,
    /// 現金股利（元）；沒有為 0。
    pub(crate) cash_dividend: Option<f64>,
    /// 股票股利（元）；沒有為 0。
    pub(crate) stock_dividend: Option<f64>,
    /// 除權息前一個交易日。
    pub(crate) pre_ex_date: String,
    /// 除權息前一個交易日的收盤價（填息目標價）。
    pub(crate) pre_ex_close: Option<f64>,
    /// 是否已填息。
    pub(crate) filled: bool,
    /// 填息日；尚未填息為 `null`。
    pub(crate) fill_date: Option<String>,
    /// 除權息日起算第幾個交易日填息（除權息當日為 1）；尚未填息為 `null`。
    pub(crate) trading_days_to_fill: Option<u32>,
    /// 除權息日到最新一筆收盤共幾個交易日（含除權息日）。
    pub(crate) trading_days_since_ex: u32,
    /// 最新一筆收盤價。
    pub(crate) latest_close: Option<f64>,
    /// 尚未填息時，最新收盤距離填息價還差多少（%）；已填息為 `null`。
    pub(crate) gap_percent: Option<f64>,
}

/// 這幾次除權息的填息統計。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct DividendFillSummary {
    /// 統計的除權息次數。
    pub(crate) events: usize,
    /// 已填息的次數。
    pub(crate) filled: usize,
    /// 已填息者的平均填息交易日數；沒有已填息者時為 `null`。
    pub(crate) average_trading_days_to_fill: Option<f64>,
    /// 已填息者的填息交易日數中位數；沒有已填息者時為 `null`。
    pub(crate) median_trading_days_to_fill: Option<f64>,
}

/// 填息分析結果。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct DividendFillResponse {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 計算所用最新一筆收盤的日期；沒有日線時為 `null`。
    pub(crate) data_as_of: Option<String>,
    /// 統計。
    pub(crate) summary: DividendFillSummary,
    /// 除權息事件，依除權息日由新到舊。
    pub(crate) events: Vec<DividendFillEvent>,
}
