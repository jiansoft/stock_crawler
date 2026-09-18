//! `/stocks/screen` 條件選股的 request、response 與 OpenAPI schema。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::MarketParamValue;

/// OpenAPI 文件使用的四種估值分類。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema。
enum ValuationBandParamValue {
    /// 不高於便宜價。
    Undervalued,
    /// 便宜價至合理價。
    FairValued,
    /// 合理價至昂貴價。
    Overvalued,
    /// 高於昂貴價。
    HighlyOvervalued,
}

/// OpenAPI 文件使用的選股排序欄位。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema。
enum StockScreenSortValue {
    /// 股票代號。
    StockSymbol,
    /// 營收年增率。
    RevenueYoy,
    /// 每股盈餘。
    Eps,
    /// 股東權益報酬率。
    Roe,
    /// 殖利率。
    DividendYield,
    /// 估值百分比。
    ValuationPercentage,
}

/// OpenAPI 文件使用的排序方向。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema。
enum SortOrderParamValue {
    /// 升冪。
    Asc,
    /// 降冪。
    Desc,
}

/// 條件選股結果中的單一股票。
///
/// 四組來源期間即使已超過新鮮度上限仍會保留，方便呼叫端判斷資料為何被
/// 轉成 `null`；過期的指標本身不參與篩選，也不會被誤當成目前數值。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct ScreenedStock {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 市場編號（上市 2、上櫃 4）。
    pub(crate) market_id: i32,
    /// 產業分類編號。
    pub(crate) industry_id: i32,
    /// 最新且仍在三個月內的營收年增率；缺值或過期時為 `null`。
    pub(crate) revenue_yoy_percent: Option<f64>,
    /// 最新且仍在兩季內的季度 EPS；缺值或過期時為 `null`。
    pub(crate) earnings_per_share: Option<f64>,
    /// 最新且仍在兩季內的季度 ROE；缺值或過期時為 `null`。
    pub(crate) return_on_equity: Option<f64>,
    /// 最新且仍在 31 天內的殖利率百分比；缺值或過期時為 `null`。
    pub(crate) dividend_yield_percent: Option<f64>,
    /// 最新且仍在 31 天內的估值區間；缺值或過期時為 `null`。
    pub(crate) valuation_band: Option<String>,
    /// 最新且仍在 31 天內的估值百分比；缺值或過期時為 `null`。
    pub(crate) valuation_percentage: Option<f64>,
    /// 該股票最新營收月份，格式 `YYYY-MM`；過期時仍保留。
    pub(crate) revenue_month: Option<String>,
    /// 該股票最新季度財報期間，格式 `YYYY-Q1`～`YYYY-Q4`；過期時仍保留。
    pub(crate) financial_period: Option<String>,
    /// 該股票最新估值日期，格式 `YYYY-MM-DD`；過期時仍保留。
    pub(crate) valuation_date: Option<String>,
    /// 該股票最新殖利率日期，格式 `YYYY-MM-DD`；過期時仍保留。
    pub(crate) yield_date: Option<String>,
}

/// 條件選股成功回應（§3.4 envelope）。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct StockScreeningResponse {
    /// 混合資料來源沒有單一正確日期，因此固定為 `null`。
    pub(crate) data_as_of: Option<String>,
    /// 通過所有固定條件的股票，最多五十筆。
    pub(crate) stocks: Vec<ScreenedStock>,
}

/// 條件選股 endpoint 的固定白名單 query string。
///
/// 所有數值欄位先在 handler 驗證範圍，再轉成 PostgreSQL `NUMERIC` 綁定值；
/// `sort_by` 與 `sort_order` 只會映射到程式內建的十二個排序分支。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct StockScreeningParams {
    /// 市場：`all`（預設）、`twse` 或 `tpex`。
    #[param(value_type = MarketParamValue, inline, default = "all")]
    pub(crate) market: Option<String>,
    /// 可選的正整數產業分類編號。
    #[param(minimum = 1)]
    pub(crate) industry_id: Option<i32>,
    /// 估值區間固定 enum。
    #[param(value_type = ValuationBandParamValue, inline)]
    pub(crate) valuation_band: Option<String>,
    /// 最低營收年增率百分比，範圍 -100–10000。
    #[param(minimum = -100, maximum = 10000)]
    pub(crate) min_revenue_yoy_percent: Option<f64>,
    /// 最低每股盈餘，範圍 -10000–10000。
    #[param(minimum = -10000, maximum = 10000)]
    pub(crate) min_eps: Option<f64>,
    /// 最低股東權益報酬率百分比，範圍 -10000–10000。
    #[param(minimum = -10000, maximum = 10000)]
    pub(crate) min_roe_percent: Option<f64>,
    /// 最低殖利率百分比，範圍 0–1000。
    #[param(minimum = 0, maximum = 1000)]
    pub(crate) min_dividend_yield_percent: Option<f64>,
    /// 排序欄位固定 enum；預設 `stock_symbol`。
    #[param(value_type = StockScreenSortValue, inline, default = "stock_symbol")]
    pub(crate) sort_by: Option<String>,
    /// 排序方向：`asc`（預設）或 `desc`。
    #[param(value_type = SortOrderParamValue, inline, default = "asc")]
    pub(crate) sort_order: Option<String>,
    /// 最多回傳筆數，預設 20，範圍 1–50。
    #[param(minimum = 1, maximum = 50, default = 20)]
    pub(crate) limit: Option<u8>,
}
