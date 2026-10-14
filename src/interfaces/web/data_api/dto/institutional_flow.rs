//! 法人買賣超排行（`/market/institutional-flow`）的 request 與 response。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::MarketParamValue;

/// OpenAPI 文件使用的法人別。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema；runtime 仍以 String 回傳精確 422。
pub(crate) enum InvestorParamValue {
    /// 外資及陸資（含外資自營商）。
    Foreign,
    /// 投信。
    Trust,
    /// 自營商（自行買賣與避險合計）。
    Dealer,
    /// 三大法人合計。
    Total,
}

/// OpenAPI 文件使用的買賣方向。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema。
pub(crate) enum FlowSideParamValue {
    /// 買超（淨買金額由大到小）。
    Buy,
    /// 賣超（淨賣金額由大到小）。
    Sell,
}

/// 法人買賣超排行的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct InstitutionalFlowParams {
    /// 交易日（`YYYY-MM-DD`）；省略為最新一個有籌碼資料的交易日，遇休市日取之前最近的交易日。
    pub(crate) date: Option<String>,
    /// 累計最近幾個有籌碼資料的交易日，預設 1，範圍 1–60。
    #[param(minimum = 1, maximum = 60, default = 1)]
    pub(crate) days: Option<u8>,
    /// 法人別：`foreign`（預設）、`trust`、`dealer` 或 `total`。
    #[param(value_type = InvestorParamValue, default = "foreign")]
    pub(crate) investor: Option<String>,
    /// `buy`（預設，買超排行）或 `sell`（賣超排行），依估計淨買賣金額排序。
    #[param(value_type = FlowSideParamValue, default = "buy")]
    pub(crate) side: Option<String>,
    /// 市場：`all`（預設）、`twse` 或 `tpex`。
    #[param(value_type = MarketParamValue, default = "all")]
    pub(crate) market: Option<String>,
    /// 產業名稱（例如 `半導體業`），省略為全部產業。
    pub(crate) industry: Option<String>,
    /// 最多回傳筆數，預設 30，範圍 1–100。
    #[param(minimum = 1, maximum = 100, default = 30)]
    pub(crate) limit: Option<u8>,
}

/// 法人買賣超排行的一檔股票。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct InstitutionalFlowItem {
    /// 名次（1 為買超或賣超金額最大）。
    pub(crate) rank: usize,
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 市場：`twse` 或 `tpex`。
    pub(crate) market: String,
    /// 產業名稱；未分類為空字串。
    pub(crate) industry: String,
    /// 期間累計淨買賣股數（正為買超）。
    pub(crate) net_shares: i64,
    /// 期間累計淨買賣張數（股數 ÷ 1000，四捨五入）。
    pub(crate) net_lots: i64,
    /// 估計淨買賣金額（元）：每日淨股數 × 當日收盤價加總。
    pub(crate) net_amount: f64,
    /// 淨買賣股數占期間成交股數的百分比；期間沒有成交量為 `null`。
    pub(crate) volume_ratio: Option<f64>,
    /// 期間內淨買超的天數。
    pub(crate) buy_days: i64,
    /// 期間內淨賣超的天數。
    pub(crate) sell_days: i64,
    /// 截至資料日的連續天數：正數為連續買超、負數為連續賣超、0 為資料日沒有買賣；
    /// 最多回看 60 個有籌碼資料的交易日。
    pub(crate) streak: i32,
    /// 資料日收盤價；當天沒有成交為 `null`。
    pub(crate) closing_price: Option<f64>,
}

/// 法人買賣超排行的成功回應。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct InstitutionalFlowResponse {
    /// 資料日（期間最後一個交易日，`YYYY-MM-DD`）。
    pub(crate) data_as_of: String,
    /// 期間第一個交易日（`YYYY-MM-DD`）。
    pub(crate) from: String,
    /// 實際累計的交易日數（資料不足時少於要求的天數）。
    pub(crate) days: usize,
    /// 法人別。
    pub(crate) investor: String,
    /// 買賣方向。
    pub(crate) side: String,
    /// 市場條件。
    pub(crate) market: String,
    /// 產業條件；未指定為 `null`。
    pub(crate) industry: Option<String>,
    /// 符合條件（含方向）的股票數。
    pub(crate) matched: usize,
    /// 依估計金額排序的股票，最多 `limit` 筆。
    pub(crate) items: Vec<InstitutionalFlowItem>,
}
