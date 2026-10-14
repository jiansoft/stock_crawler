//! `/stocks/{symbol}/technicals` 的 request、response 與 OpenAPI schema。
//!
//! 均線、乖離率、RSI、KD、MACD 與最新一日的技術訊號；計算規則見
//! [`crate::domain::quote::technical`]。價格單位為新台幣元，已還原權值（最新一日為實際價格）。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// 技術指標 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct TechnicalIndicatorsParams {
    /// 截止交易日（`YYYY-MM-DD`）；省略為最新交易日，遇休市日取之前最近的交易日。
    pub(crate) date: Option<String>,
    /// 回傳最近幾個交易日的指標，預設 20，範圍 1–120。
    #[param(minimum = 1, maximum = 120, default = 20)]
    pub(crate) days: Option<u8>,
}

/// 單日的技術指標；樣本不足的欄位為 `null`，數值四捨五入到小數兩位。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct TechnicalPoint {
    /// 交易日。
    pub(crate) date: String,
    /// 收盤價。
    pub(crate) close: f64,
    /// 5 日均線（週線）。
    pub(crate) ma5: Option<f64>,
    /// 10 日均線。
    pub(crate) ma10: Option<f64>,
    /// 20 日均線（月線）。
    pub(crate) ma20: Option<f64>,
    /// 60 日均線（季線）。
    pub(crate) ma60: Option<f64>,
    /// 120 日均線（半年線）。
    pub(crate) ma120: Option<f64>,
    /// 240 日均線（年線）。
    pub(crate) ma240: Option<f64>,
    /// 收盤相對 5 日均線的乖離率（%）。
    pub(crate) bias5: Option<f64>,
    /// 收盤相對 20 日均線的乖離率（%）。
    pub(crate) bias20: Option<f64>,
    /// 收盤相對 60 日均線的乖離率（%）。
    pub(crate) bias60: Option<f64>,
    /// 收盤相對 240 日均線的乖離率（%）。
    pub(crate) bias240: Option<f64>,
    /// RSI(6)。
    pub(crate) rsi6: Option<f64>,
    /// RSI(12)。
    pub(crate) rsi12: Option<f64>,
    /// RSI(14)。
    pub(crate) rsi14: Option<f64>,
    /// KD(9,3,3) 的 K 值。
    pub(crate) k: Option<f64>,
    /// KD(9,3,3) 的 D 值。
    pub(crate) d: Option<f64>,
    /// MACD(12,26,9) 的 DIF（收盤價 EMA12 − EMA26）。
    pub(crate) macd_dif: Option<f64>,
    /// MACD 訊號線（DIF 的 9 日 EMA）。
    pub(crate) macd_signal: Option<f64>,
    /// MACD 柱狀體（DIF − 訊號線）。
    pub(crate) macd_histogram: Option<f64>,
}

/// 最新一日的技術訊號。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct TechnicalSignalItem {
    /// 訊號來源：`ma5_ma20`、`ma20_ma60`、`kd`、`macd`、`close_ma20`、`close_ma60`、
    /// `close_ma240` 或 `rsi14`。
    pub(crate) indicator: String,
    /// 訊號種類：`golden_cross`、`death_cross`、`cross_above`、`cross_below`、
    /// `overbought` 或 `oversold`。
    pub(crate) kind: String,
    /// 中文說明。
    pub(crate) description: String,
}

/// 技術指標 endpoint 的完整回應。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct TechnicalIndicatorsResponse {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 最新一筆指標的交易日；截止日前沒有任何日線時為 `null`。
    pub(crate) data_as_of: Option<String>,
    /// 請求的天數。
    pub(crate) days: u8,
    /// 價格是否還原權值（固定為 `true`：除權息、減資、分割之前的價格依參考價調整，最新一日為實際價格）。
    pub(crate) adjusted: bool,
    /// 由舊到新的每日指標，最多 `days` 筆。
    pub(crate) points: Vec<TechnicalPoint>,
    /// 最新一日出現的交叉與超買超賣訊號。
    pub(crate) signals: Vec<TechnicalSignalItem>,
}
