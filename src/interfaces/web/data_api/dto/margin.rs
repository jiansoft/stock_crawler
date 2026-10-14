//! 融資融券排行（`/market/margin-ranking`）的 request 與 response。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::MarketParamValue;

/// OpenAPI 文件使用的融資融券排序鍵。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema；runtime 仍以 String 回傳精確 422。
pub(crate) enum MarginRankByValue {
    /// 融資增加最多。
    MarginIncrease,
    /// 融資減少最多。
    MarginDecrease,
    /// 融券增加最多。
    ShortIncrease,
    /// 融券減少最多。
    ShortDecrease,
    /// 券資比最高（融資餘額至少 100 張）。
    ShortMarginRatio,
}

/// 融資融券排行的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct MarginRankingParams {
    /// 交易日（`YYYY-MM-DD`）；省略為最新一個有籌碼資料的交易日，遇休市日取之前最近的交易日。
    pub(crate) date: Option<String>,
    /// 比較最近幾個有籌碼資料的交易日的增減，預設 1，範圍 1–60。
    #[param(minimum = 1, maximum = 60, default = 1)]
    pub(crate) days: Option<u8>,
    /// 排序鍵：`margin_increase`（預設）、`margin_decrease`、`short_increase`、`short_decrease`
    /// 或 `short_margin_ratio`。
    #[param(value_type = MarginRankByValue, default = "margin_increase")]
    pub(crate) rank_by: Option<String>,
    /// 市場：`all`（預設）、`twse` 或 `tpex`。
    #[param(value_type = MarketParamValue, default = "all")]
    pub(crate) market: Option<String>,
    /// 產業名稱（例如 `半導體業`），省略為全部產業。
    pub(crate) industry: Option<String>,
    /// 最多回傳筆數，預設 30，範圍 1–100。
    #[param(minimum = 1, maximum = 100, default = 30)]
    pub(crate) limit: Option<u8>,
}

/// 融資融券排行的一檔股票；單位皆為張。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct MarginRankItem {
    /// 名次。
    pub(crate) rank: usize,
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 市場：`twse` 或 `tpex`。
    pub(crate) market: String,
    /// 產業名稱；未分類為空字串。
    pub(crate) industry: String,
    /// 資料日融資餘額（張）。
    pub(crate) margin_balance: i64,
    /// 期間融資增減（張）：資料日餘額減期間第一天的前日餘額。
    pub(crate) margin_change: i64,
    /// 期間融資增減百分比；期初餘額為 0 時為 `null`。
    pub(crate) margin_change_percent: Option<f64>,
    /// 資料日融券餘額（張）。
    pub(crate) short_balance: i64,
    /// 期間融券增減（張）。
    pub(crate) short_change: i64,
    /// 券資比（融券餘額 ÷ 融資餘額，%）；融資餘額為 0 時為 `null`。
    pub(crate) short_margin_ratio: Option<f64>,
    /// 資料日收盤價；當天沒有成交為 `null`。
    pub(crate) closing_price: Option<f64>,
}

/// 融資融券排行的成功回應。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct MarginRankingResponse {
    /// 資料日（期間最後一個交易日，`YYYY-MM-DD`）。
    pub(crate) data_as_of: String,
    /// 期間第一個交易日（`YYYY-MM-DD`）。
    pub(crate) from: String,
    /// 實際比較的交易日數。
    pub(crate) days: usize,
    /// 排序鍵。
    pub(crate) rank_by: String,
    /// 市場條件。
    pub(crate) market: String,
    /// 產業條件；未指定為 `null`。
    pub(crate) industry: Option<String>,
    /// 符合條件（含排序鍵的方向）的股票數。
    pub(crate) matched: usize,
    /// 排行，最多 `limit` 筆。
    pub(crate) items: Vec<MarginRankItem>,
}
