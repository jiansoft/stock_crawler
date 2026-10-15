//! 法人買賣超排行（`/market/institutional-flow`）的 request 與 response。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{
    BenchmarkStat, BenchmarkYear, GroupByParamValue, HorizonStat, MarketParamValue,
    PerformanceYear, SortOrderParamValue,
};

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

/// 產業法人資金流向的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndustryFlowParams {
    /// 交易日（`YYYY-MM-DD`）；省略為最新一個有籌碼資料的交易日，遇休市日取之前最近的交易日。
    pub(crate) date: Option<String>,
    /// 累計最近幾個有籌碼資料的交易日，預設 5，範圍 1–60。
    #[param(minimum = 1, maximum = 60, default = 5)]
    pub(crate) days: Option<u8>,
    /// 法人別：`foreign`（預設）、`trust`、`dealer` 或 `total`。
    #[param(value_type = InvestorParamValue, default = "foreign")]
    pub(crate) investor: Option<String>,
    /// 市場：`all`（預設）、`twse` 或 `tpex`。
    #[param(value_type = MarketParamValue, default = "all")]
    pub(crate) market: Option<String>,
    /// `desc`（預設，淨流入最多的產業在前）或 `asc`（淨流出最多的產業在前）。
    #[param(value_type = SortOrderParamValue, default = "desc")]
    pub(crate) order: Option<String>,
}

/// 產業內法人淨買賣金額最大的個股。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct IndustryFlowStock {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 估計淨買賣金額（元，正為買超）。
    pub(crate) net_amount: f64,
}

/// 單一產業的法人資金流向。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct IndustryFlowItem {
    /// 依淨買賣金額由多到少的名次（1 為淨流入最多）。
    pub(crate) rank: usize,
    /// 產業名稱。
    pub(crate) industry: String,
    /// 期間內有法人買賣資料的股票數。
    pub(crate) stocks: usize,
    /// 淨買超的股票數。
    pub(crate) buy_stocks: usize,
    /// 淨賣超的股票數。
    pub(crate) sell_stocks: usize,
    /// 產業合計估計淨買賣金額（元）。
    pub(crate) net_amount: f64,
    /// 淨買賣金額占產業期間成交金額的百分比；沒有成交金額為 `null`。
    pub(crate) trade_value_ratio: Option<f64>,
    /// 淨買超金額最大的個股（最多 3 檔）。
    pub(crate) top_buyers: Vec<IndustryFlowStock>,
    /// 淨賣超金額最大的個股（最多 3 檔）。
    pub(crate) top_sellers: Vec<IndustryFlowStock>,
}

/// 產業法人資金流向的成功回應。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct IndustryFlowResponse {
    /// 資料日（期間最後一個交易日，`YYYY-MM-DD`）。
    pub(crate) data_as_of: String,
    /// 期間第一個交易日（`YYYY-MM-DD`）。
    pub(crate) from: String,
    /// 實際累計的交易日數。
    pub(crate) days: usize,
    /// 法人別。
    pub(crate) investor: String,
    /// 市場條件。
    pub(crate) market: String,
    /// 排序方向。
    pub(crate) order: String,
    /// 所有產業（含未分類股票）合計的估計淨買賣金額（元）。
    pub(crate) total_net_amount: f64,
    /// 各產業，依 `order` 排序。
    pub(crate) items: Vec<IndustryFlowItem>,
}

/// 法人連續買賣超後報酬統計的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct InstitutionalStreakPerformanceParams {
    /// 統計起日（`YYYY-MM-DD`）；省略為迄日往前 364 天。
    pub(crate) from: Option<String>,
    /// 統計迄日（`YYYY-MM-DD`）；省略為最新有籌碼資料的交易日。起訖最長 3,653 天（籌碼資料從 2019 年起）。
    pub(crate) to: Option<String>,
    /// 法人別：`foreign`（預設）、`trust`、`dealer` 或 `total`。
    #[param(value_type = InvestorParamValue, default = "foreign")]
    pub(crate) investor: Option<String>,
    /// `buy`（預設，連續買超）或 `sell`（連續賣超）。
    #[param(value_type = FlowSideParamValue, default = "buy")]
    pub(crate) side: Option<String>,
    /// 連續天數 N（1–20，預設 3）：某檔股票連續買超（或賣超）剛好滿 N 天的那一天算一次事件，
    /// 之後延續的天數不重複計算。
    #[param(minimum = 1, maximum = 20, default = 3)]
    pub(crate) streak: Option<u8>,
    /// 市場：`all`（預設）、`twse` 或 `tpex`；大盤基準也依此市場計算。
    #[param(value_type = MarketParamValue, default = "all")]
    pub(crate) market: Option<String>,
    /// 只統計這個產業（產業名稱，例如「半導體業」）的事件；大盤基準不受影響。
    pub(crate) industry: Option<String>,
    /// `year`：除了整段期間，再依事件日的年度拆分統計（`years`、`benchmark_by_year`）；省略為不拆分。
    #[param(value_type = Option<GroupByParamValue>)]
    pub(crate) group_by: Option<String>,
}

/// 法人連續買賣超後報酬統計的成功回應。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct InstitutionalStreakPerformanceResponse {
    /// 統計起日。
    pub(crate) from: String,
    /// 統計迄日。
    pub(crate) to: String,
    /// 市場：`all`、`twse` 或 `tpex`。
    pub(crate) market: String,
    /// 法人別。
    pub(crate) investor: String,
    /// 買賣方向。
    pub(crate) side: String,
    /// 連續天數 N。
    pub(crate) streak: u8,
    /// 篩選的產業；未篩選為 `null`。
    pub(crate) industry: Option<String>,
    /// 拆分方式：`year` 或未拆分為 `null`。
    pub(crate) group_by: Option<String>,
    /// 報酬是否還原權值；固定為 `true`。
    pub(crate) adjusted: bool,
    /// 期間內的事件數（連續滿 N 天的次數）。
    pub(crate) occurrences: i64,
    /// 事件之後 5／20／60／120／240 個交易日的表現。
    pub(crate) horizons: Vec<HorizonStat>,
    /// `group_by=year` 時各年度的表現，由舊到新（只列有事件的年度）；未拆分為 `null`。
    pub(crate) years: Option<Vec<PerformanceYear>>,
    /// 大盤基準，依天期由短到長。
    pub(crate) benchmark: Vec<BenchmarkStat>,
    /// `group_by=year` 時各年度的大盤基準，由舊到新；未拆分為 `null`。
    pub(crate) benchmark_by_year: Option<Vec<BenchmarkYear>>,
}
