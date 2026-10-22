//! ETF 專區（`/etfs`、`/etfs/{symbol}`）的 request 與 response。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::MarketParamValue;

/// OpenAPI 文件使用的 ETF 分類代碼。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema；runtime 仍以 String 回傳精確 422。
pub(crate) enum EtfStrategyValue {
    /// 市值型。
    MarketCap,
    /// 高股息。
    HighDividend,
    /// 主動式。
    Active,
    /// 債券。
    Bond,
    /// 槓桿。
    Leveraged,
    /// 反向。
    Inverse,
    /// 期貨商品。
    Commodity,
    /// 海外股票。
    Overseas,
    /// 主題。
    Thematic,
}

/// OpenAPI 文件使用的 ETF 清單排序鍵。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema；runtime 仍以 String 回傳精確 422。
pub(crate) enum EtfSortValue {
    /// 成交值。
    TradeValue,
    /// 資產規模。
    TotalAssets,
    /// 折溢價。
    PremiumRate,
    /// 近 12 個月殖利率。
    #[schema(rename = "yield_12m")]
    Yield12m,
    /// 淨申購金額。
    NetFlow,
    /// 近 1 月含息報酬。
    #[schema(rename = "ret_1m")]
    Ret1m,
    /// 今年以來含息報酬。
    RetYtd,
    /// 近 1 年含息報酬。
    #[schema(rename = "ret_1y")]
    Ret1y,
    /// 受益人數。
    Holders,
}

/// OpenAPI 文件使用的配息頻率。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema；runtime 仍以 String 回傳精確 422。
pub(crate) enum EtfFrequencyValue {
    /// 月配。
    Monthly,
    /// 季配。
    Quarterly,
    /// 半年配。
    SemiAnnual,
    /// 年配。
    Annual,
    /// 不配息。
    None,
}

/// ETF 清單的 query string。
#[derive(Debug, Default, Deserialize, IntoParams)]
pub(crate) struct EtfListParams {
    /// 分類代碼（見 `EtfStrategyValue`），多個以逗號分隔（例如 `bond,high_dividend`）；省略為全部。
    pub(crate) strategy: Option<String>,
    /// 投資地區（例如 `台灣`、`美國`、`全球`）；省略為全部。
    pub(crate) region: Option<String>,
    /// 配息頻率；省略為全部。
    #[param(value_type = EtfFrequencyValue)]
    pub(crate) frequency: Option<String>,
    /// 市場：`all`（預設）、`twse` 或 `tpex`。
    #[param(value_type = MarketParamValue, default = "all")]
    pub(crate) market: Option<String>,
    /// 關鍵字：比對代號、名稱、全名、發行投信、追蹤指數與題材（不分大小寫，最多 40 字）。
    pub(crate) keyword: Option<String>,
    /// 排序鍵，預設 `trade_value`。
    #[param(value_type = EtfSortValue, default = "trade_value")]
    pub(crate) sort: Option<String>,
    /// `desc`（預設，由大到小）或 `asc`。
    pub(crate) order: Option<String>,
    /// 最多回傳筆數，預設 20，範圍 1–100。
    #[param(minimum = 1, maximum = 100, default = 20)]
    pub(crate) limit: Option<u8>,
}

/// ETF 明細的 query string。
#[derive(Debug, Default, Deserialize, IntoParams)]
pub(crate) struct EtfDetailParams {
    /// 近期每日淨值與折溢價的筆數，預設 20，範圍 1–60。
    #[param(minimum = 1, maximum = 60, default = 20)]
    pub(crate) nav_days: Option<u8>,
}

/// 一檔 ETF 的概況；比率為 %，金額為元。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct EtfItem {
    /// ETF 代號。
    pub(crate) stock_symbol: String,
    /// 名稱。
    pub(crate) name: String,
    /// 市場：`twse` 或 `tpex`。
    pub(crate) market: String,
    /// 分類代碼（`market_cap`、`high_dividend`、`active`、`bond`、`leveraged`、`inverse`、`commodity`、`overseas`、`thematic`）。
    pub(crate) strategy: String,
    /// 投資地區。
    pub(crate) region: String,
    /// 題材。
    pub(crate) themes: Vec<String>,
    /// 配息頻率（`monthly`、`quarterly`、`semi_annual`、`annual`、`none`；未知為空字串）。
    pub(crate) dividend_frequency: String,
    /// 發行投信。
    pub(crate) issuer: String,
    /// 追蹤指數。
    pub(crate) benchmark: String,
    /// 行情日（`YYYY-MM-DD`）。
    pub(crate) quote_date: Option<String>,
    /// 收盤價。
    pub(crate) closing_price: Option<f64>,
    /// 漲跌幅（%）。
    pub(crate) change_range: Option<f64>,
    /// 成交值（元）。
    pub(crate) trade_value: Option<f64>,
    /// 淨值日期（`YYYY-MM-DD`）。
    pub(crate) nav_date: Option<String>,
    /// 投信預估淨值。
    pub(crate) nav: Option<f64>,
    /// 折溢價（%）：(市價 − 淨值) ÷ 淨值。
    pub(crate) premium_rate: Option<f64>,
    /// 近 20 筆折溢價平均（%）。
    pub(crate) premium_avg_20d: Option<f64>,
    /// 資產規模（元）：已發行單位 × 淨值。
    pub(crate) total_assets: Option<f64>,
    /// 淨申購金額（元）：單位增減 × 淨值；正為淨流入。
    pub(crate) net_flow: Option<f64>,
    /// 近 12 個月現金配息合計（元）。
    pub(crate) dividend_12m: Option<f64>,
    /// 近 12 個月殖利率（%）。
    pub(crate) yield_12m: Option<f64>,
    /// 下次除息日（`YYYY-MM-DD`）。
    pub(crate) next_ex_date: Option<String>,
    /// 下次配息金額（元）。
    pub(crate) next_dividend: Option<f64>,
    /// 近 1 月市價含息報酬（%）。
    pub(crate) ret_1m: Option<f64>,
    /// 今年以來市價含息報酬（%）。
    pub(crate) ret_ytd: Option<f64>,
    /// 近 1 年市價含息報酬（%）。
    pub(crate) ret_1y: Option<f64>,
    /// 集保受益人數。
    pub(crate) holders: Option<i64>,
    /// 外資買賣超（股）。
    pub(crate) foreign_net: Option<i64>,
}

/// ETF 清單。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct EtfListResponse {
    /// 符合條件的總檔數（不受 `limit` 限制）。
    pub(crate) total: i64,
    /// 實際使用的排序鍵。
    pub(crate) sort: String,
    /// 實際使用的排序方向（`desc` 或 `asc`）。
    pub(crate) order: String,
    /// ETF 清單。
    pub(crate) items: Vec<EtfItem>,
}

/// ETF 基本資料。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct EtfProfileInfo {
    /// 基金全名。
    pub(crate) full_name: String,
    /// 成立日（`YYYY-MM-DD`）。
    pub(crate) inception_date: Option<String>,
    /// 是否為主動式。
    pub(crate) is_active: bool,
    /// 經理費（來源原文）。
    pub(crate) management_fee: String,
    /// 保管費（來源原文）。
    pub(crate) custodian_fee: String,
    /// 計價幣別。
    pub(crate) currency: String,
    /// 資料來源：`yahoo` 或 `heuristic`（只由代號與名稱推斷）。
    pub(crate) source: String,
}

/// ETF 績效；報酬為 %，3 年以上年化。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct EtfPerformanceInfo {
    /// 市價含息報酬的基準日（`YYYY-MM-DD`）。
    pub(crate) calc_date: Option<String>,
    /// 市價含息報酬：1 週。
    pub(crate) ret_1w: Option<f64>,
    /// 1 個月。
    pub(crate) ret_1m: Option<f64>,
    /// 3 個月。
    pub(crate) ret_3m: Option<f64>,
    /// 6 個月。
    pub(crate) ret_6m: Option<f64>,
    /// 今年以來。
    pub(crate) ret_ytd: Option<f64>,
    /// 1 年。
    pub(crate) ret_1y: Option<f64>,
    /// 3 年年化。
    pub(crate) ret_3y: Option<f64>,
    /// 5 年年化。
    pub(crate) ret_5y: Option<f64>,
    /// 近 1 年年化波動（%）。
    pub(crate) vol_1y: Option<f64>,
    /// 近 1 年最大回撤（%，負值）。
    pub(crate) mdd_1y: Option<f64>,
    /// 近 1 年對 0050 的 beta。
    pub(crate) beta_1y: Option<f64>,
    /// 近 1 年夏普值（未扣無風險利率）。
    pub(crate) sharpe_1y: Option<f64>,
    /// 淨值含息報酬的基準日（`YYYY-MM-DD`）。
    pub(crate) nav_calc_date: Option<String>,
    /// 淨值含息報酬：1 個月。
    pub(crate) nav_ret_1m: Option<f64>,
    /// 淨值含息報酬：今年以來。
    pub(crate) nav_ret_ytd: Option<f64>,
    /// 淨值含息報酬：1 年。
    pub(crate) nav_ret_1y: Option<f64>,
    /// 淨值含息報酬：3 年年化。
    pub(crate) nav_ret_3y: Option<f64>,
}

/// 前十大持股的一列。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct EtfHoldingItem {
    /// 台股成分股代號；債券等沒有代號為空字串。
    pub(crate) symbol: String,
    /// 名稱。
    pub(crate) name: String,
    /// 比例（%）。
    pub(crate) weight: f64,
}

/// 行業比重或資產分佈的一列。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct EtfAllocationItem {
    /// 行業別或資產類別。
    pub(crate) label: String,
    /// 比例（%）。
    pub(crate) weight: f64,
}

/// 近期一天的淨值與折溢價。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct EtfNavPoint {
    /// 日期（`YYYY-MM-DD`）。
    pub(crate) date: String,
    /// 淨值（正式淨值優先，當天為預估淨值）。
    pub(crate) nav: Option<f64>,
    /// 市價。
    pub(crate) market_price: Option<f64>,
    /// 折溢價（%）。
    pub(crate) premium_rate: Option<f64>,
    /// 受益單位增減；歷史回補的日子沒有，為 `null`。
    pub(crate) units_change: Option<i64>,
}

/// ETF 明細。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct EtfDetailResponse {
    /// 概況。
    pub(crate) item: EtfItem,
    /// 基本資料；沒有時為 `null`。
    pub(crate) profile: Option<EtfProfileInfo>,
    /// 績效；上市未滿一週或沒有成交時為 `null`。
    pub(crate) performance: Option<EtfPerformanceInfo>,
    /// 前十大持股的資料日期；沒有時為 `null`。
    pub(crate) holdings_as_of: Option<String>,
    /// 前十大持股（反向、期貨型通常沒有）。
    pub(crate) holdings: Vec<EtfHoldingItem>,
    /// 行業比重的資料日期。
    pub(crate) industries_as_of: Option<String>,
    /// 行業比重。
    pub(crate) industries: Vec<EtfAllocationItem>,
    /// 資產分佈的資料日期。
    pub(crate) assets_as_of: Option<String>,
    /// 資產分佈。
    pub(crate) assets: Vec<EtfAllocationItem>,
    /// 近期每日淨值與折溢價，由新到舊。
    pub(crate) navs: Vec<EtfNavPoint>,
}

/// ETF 比較的 query string。
#[derive(Debug, Default, Deserialize, IntoParams)]
pub(crate) struct EtfCompareParams {
    /// 要比較的 ETF 代號，2 至 4 檔，以逗號分隔（例如 `0050,006208`）。
    pub(crate) symbols: Option<String>,
}

/// 比較表的一檔 ETF；比率為 %，金額為元，報酬 3 年以上年化。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct EtfCompareItem {
    /// ETF 代號。
    pub(crate) stock_symbol: String,
    /// 名稱。
    pub(crate) name: String,
    /// 分類代碼。
    pub(crate) strategy: String,
    /// 追蹤指數。
    pub(crate) benchmark: String,
    /// 經理費（來源原文）；沒有基本資料時為空字串。
    pub(crate) management_fee: String,
    /// 保管費（來源原文）；沒有基本資料時為空字串。
    pub(crate) custodian_fee: String,
    /// 資產規模（元）。
    pub(crate) total_assets: Option<f64>,
    /// 收盤價。
    pub(crate) closing_price: Option<f64>,
    /// 折溢價（%）。
    pub(crate) premium_rate: Option<f64>,
    /// 近 20 筆折溢價平均（%）。
    pub(crate) premium_avg_20d: Option<f64>,
    /// 近 12 個月殖利率（%）。
    pub(crate) yield_12m: Option<f64>,
    /// 配息頻率。
    pub(crate) dividend_frequency: String,
    /// 近 1 年市價含息報酬（%）。
    pub(crate) ret_1y: Option<f64>,
    /// 3 年年化市價含息報酬（%）。
    pub(crate) ret_3y: Option<f64>,
    /// 近 1 年淨值含息報酬（%）。
    pub(crate) nav_ret_1y: Option<f64>,
    /// 近 1 年年化波動（%）。
    pub(crate) vol_1y: Option<f64>,
    /// 近 1 年最大回撤（%，負值）。
    pub(crate) mdd_1y: Option<f64>,
    /// 近 1 年夏普值。
    pub(crate) sharpe_1y: Option<f64>,
    /// 前十大持股的資料日期。
    pub(crate) holdings_as_of: Option<String>,
}

/// 兩檔 ETF 前十大持股的重疊。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct EtfOverlap {
    /// 第一檔代號。
    pub(crate) a: String,
    /// 第二檔代號。
    pub(crate) b: String,
    /// 兩檔前十大都有的持股（依重疊權重由大到小）。
    pub(crate) common: Vec<String>,
    /// 重疊權重（%）：共同持股取兩檔比例較小者加總；只看前十大，實際重疊只會更高。
    pub(crate) overlap_weight: f64,
}

/// ETF 比較。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct EtfCompareResponse {
    /// 依查詢順序的各檔資料。
    pub(crate) items: Vec<EtfCompareItem>,
    /// 兩兩之間的前十大持股重疊（任一檔沒有前十大時不列）。
    pub(crate) overlaps: Vec<EtfOverlap>,
}
