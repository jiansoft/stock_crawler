//! `/market/signals` 的 request、response 與 OpenAPI schema。
//!
//! 收盤後預先計算的全市場技術訊號；計算規則見 [`crate::domain::quote::technical`]，
//! 與 `/stocks/{symbol}/technicals` 同一套。價格未還原除權息。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::MarketParamValue;

/// OpenAPI 文件使用的訊號種類。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema；runtime 仍以 String 回傳精確 422。
pub(crate) enum SignalKindValue {
    /// 短線向上穿越長線（均線、KD、MACD）。
    GoldenCross,
    /// 短線向下穿越長線。
    DeathCross,
    /// 收盤價站上均線。
    CrossAbove,
    /// 收盤價跌破均線。
    CrossBelow,
    /// RSI(14) ≥ 70 或 K ≥ 80。
    Overbought,
    /// RSI(14) ≤ 30 或 K ≤ 20。
    Oversold,
    /// 當日最高價創 52 週新高。
    NewHigh,
    /// 當日最低價創 52 週新低。
    NewLow,
}

/// OpenAPI 文件使用的訊號來源。
#[derive(ToSchema)]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema；runtime 仍以 String 回傳精確 422。
pub(crate) enum SignalIndicatorValue {
    /// 5 日與 20 日均線交叉。
    #[schema(rename = "ma5_ma20")]
    Ma5Ma20,
    /// 20 日與 60 日均線交叉。
    #[schema(rename = "ma20_ma60")]
    Ma20Ma60,
    /// KD 交叉或 K 值超買超賣。
    #[schema(rename = "kd")]
    Kd,
    /// MACD 的 DIF 與訊號線交叉。
    #[schema(rename = "macd")]
    Macd,
    /// 收盤價站上或跌破 20 日均線（月線）。
    #[schema(rename = "close_ma20")]
    CloseMa20,
    /// 收盤價站上或跌破 60 日均線（季線）。
    #[schema(rename = "close_ma60")]
    CloseMa60,
    /// 收盤價站上或跌破 240 日均線（年線）。
    #[schema(rename = "close_ma240")]
    CloseMa240,
    /// RSI(14) 超買超賣。
    #[schema(rename = "rsi14")]
    Rsi14,
    /// 52 週新高新低。
    #[schema(rename = "52_week")]
    FiftyTwoWeek,
}

/// 技術訊號選股的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct MarketSignalsParams {
    /// 交易日（`YYYY-MM-DD`）；省略為最新交易日，遇休市日取之前最近的交易日。
    pub(crate) date: Option<String>,
    /// 市場：`all`（預設）、`twse` 或 `tpex`。
    #[param(value_type = MarketParamValue, default = "all")]
    pub(crate) market: Option<String>,
    /// 只列出這種訊號；省略為全部。
    #[param(value_type = Option<SignalKindValue>)]
    pub(crate) kind: Option<String>,
    /// 只列出這個來源的訊號；省略為全部。
    #[param(value_type = Option<SignalIndicatorValue>)]
    pub(crate) indicator: Option<String>,
    /// 只列出這個產業（產業名稱，例如「半導體業」）；省略為全部。
    pub(crate) industry: Option<String>,
    /// 最多回傳幾筆，預設 50，範圍 1–200；依成交值由大到小。
    #[param(minimum = 1, maximum = 200, default = 50)]
    pub(crate) limit: Option<u8>,
}

/// 同一來源與種類的訊號家數。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct SignalCount {
    /// 訊號來源。
    pub(crate) indicator: String,
    /// 訊號種類。
    pub(crate) kind: String,
    /// 家數。
    pub(crate) count: usize,
}

/// 出現訊號的股票。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct SignalStock {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 市場：`twse` 或 `tpex`。
    pub(crate) market: String,
    /// 產業名稱；未分類時為空字串。
    pub(crate) industry: String,
    /// 收盤價（元）。
    pub(crate) closing_price: Option<f64>,
    /// 漲跌幅（%）。
    pub(crate) change_percent: Option<f64>,
    /// 成交值（元）。
    pub(crate) trade_value: Option<f64>,
    /// 訊號來源。
    pub(crate) indicator: String,
    /// 訊號種類。
    pub(crate) kind: String,
    /// 中文說明。
    pub(crate) description: String,
}

/// 技術訊號選股結果。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct MarketSignalsResponse {
    /// 資料日。
    pub(crate) data_as_of: String,
    /// 查詢的市場。
    pub(crate) market: String,
    /// 篩選的訊號種類；未篩選為 `null`。
    pub(crate) kind: Option<String>,
    /// 篩選的訊號來源；未篩選為 `null`。
    pub(crate) indicator: Option<String>,
    /// 篩選的產業；未篩選為 `null`。
    pub(crate) industry: Option<String>,
    /// 價格是否還原除權息；固定為 `false`。
    pub(crate) adjusted: bool,
    /// 符合篩選條件的訊號筆數（未套用 `limit`）。
    pub(crate) total: usize,
    /// 同市場、同產業範圍內各來源與種類的家數（不受 `kind`、`indicator` 影響）。
    pub(crate) summary: Vec<SignalCount>,
    /// 符合條件的訊號，依成交值由大到小、同值依代號；最多 `limit` 筆。
    pub(crate) items: Vec<SignalStock>,
}
