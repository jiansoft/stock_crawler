//! 大盤趨勢狀態（`/market/regime`）的 request 與 response。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// 大盤趨勢狀態的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct MarketRegimeParams {
    /// 交易日（`YYYY-MM-DD`）；省略為最新一個有加權指數的交易日，遇休市日取之前最近的交易日。
    pub(crate) date: Option<String>,
}

/// 加權指數相對一條均線的位置。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct MovingAverageStatus {
    /// 均線天數（交易日）：5、20、60、120 或 240。
    pub(crate) days: u16,
    /// 均線值；資料不足這麼多個交易日為 `null`。
    pub(crate) value: Option<f64>,
    /// 收盤高於均線的百分比（負值為低於）。
    pub(crate) distance_percent: Option<f64>,
    /// 收盤是否不低於均線。
    pub(crate) above: Option<bool>,
    /// 均線是否比 5 個交易日前高（均線上揚）。
    pub(crate) rising: Option<bool>,
}

/// 大盤趨勢狀態。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct MarketRegimeResponse {
    /// 資料日（加權指數最新一個交易日）。
    pub(crate) data_as_of: String,
    /// 加權指數收盤。
    pub(crate) close: f64,
    /// 與前一個交易日相比的漲跌點數；沒有前一日為 `null`。
    pub(crate) change: Option<f64>,
    /// 漲跌幅（%）。
    pub(crate) change_percent: Option<f64>,
    /// 均線排列：`bullish`（收盤 > 20 > 60 > 120 > 240 日均線）、`bearish`（完全相反）或
    /// `mixed`（其他，或均線資料不足）。
    pub(crate) alignment: String,
    /// 5／20／60／120／240 日均線的位置。
    pub(crate) moving_averages: Vec<MovingAverageStatus>,
    /// 收盤連續站在 60 日均線同一側（上方或下方）的交易日數，含資料日；資料不足時只數到有均線的日子。
    pub(crate) days_on_side_of_ma60: Option<i64>,
    /// 近 250 個交易日（約 52 週）的最高收盤。
    pub(crate) high_52_week: f64,
    /// 近 250 個交易日的最低收盤。
    pub(crate) low_52_week: f64,
    /// 收盤距 52 週高點的百分比（0 或負值）。
    pub(crate) from_high_percent: f64,
    /// 收盤距 52 週低點的百分比（0 或正值）。
    pub(crate) from_low_percent: f64,
    /// 近 20 個交易日的漲跌幅（%）；資料不足為 `null`。
    pub(crate) return_20d: Option<f64>,
    /// 近 60 個交易日的漲跌幅（%）。
    pub(crate) return_60d: Option<f64>,
    /// 近 240 個交易日的漲跌幅（%）。
    pub(crate) return_240d: Option<f64>,
}
