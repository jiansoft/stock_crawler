//! `/market/*` 相關的 request、response 與 OpenAPI schema。
//!
//! 涵蓋市場廣度、殖利率排行、大盤指數歷史、股利行事曆與 QFII 持股排行。
//! 所有缺值欄位皆保留 `Option`，讓 serde 輸出 JSON `null` 而非猜測成零值。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::MarketParamValue;

/// OpenAPI 文件使用的股利行事曆事件類型（§4.9）。
///
/// 台股領域背景：一筆股利公告有四個關鍵日期——「除息日」（買進後不再
/// 享有現金股利的基準日）、「除權日」（股票股利的基準日）、「現金股利
/// 發放日」與「股票股利發放日」。行事曆把每個日期各自視為一個事件。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema；runtime 仍以 String 回傳精確 422。
enum CalendarEventTypeValue {
    /// 除息日事件。
    ExDividend,
    /// 除權日事件。
    ExRights,
    /// 現金股利發放日事件。
    CashPayable,
    /// 股票股利發放日事件。
    StockPayable,
    /// 四種事件全部回傳。
    All,
}

/// OpenAPI 文件使用的 QFII 排行排序欄位（§4.10）。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema。
enum QfiiSortValue {
    /// 依外資持股比例排序。
    Percentage,
    /// 依外資持股股數排序。
    Shares,
}

/// 單一交易日的市場廣度統計。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct MarketBreadth {
    /// 統計日期，格式 `YYYY-MM-DD`。
    pub(crate) date: String,
    /// 市場名稱：`all`、`twse` 或 `tpex`。
    pub(crate) market: String,
    /// 股價不高於便宜價的家數。
    pub(crate) undervalued: i32,
    /// 股價介於便宜價與合理價的家數。
    pub(crate) fair_valued: i32,
    /// 股價介於合理價與昂貴價的家數。
    pub(crate) overvalued: i32,
    /// 股價高於昂貴價的家數。
    pub(crate) highly_overvalued: i32,
    /// 股價不高於五日均線的家數。
    pub(crate) below_5_day_moving_average: i32,
    /// 股價高於五日均線的家數。
    pub(crate) above_5_day_moving_average: i32,
    /// 股價不高於二十日均線的家數。
    pub(crate) below_20_day_moving_average: i32,
    /// 股價高於二十日均線的家數。
    pub(crate) above_20_day_moving_average: i32,
    /// 股價不高於六十日均線的家數。
    pub(crate) below_60_day_moving_average: i32,
    /// 股價高於六十日均線的家數。
    pub(crate) above_60_day_moving_average: i32,
    /// 股價不高於一百二十日均線的家數。
    pub(crate) below_120_day_moving_average: i32,
    /// 股價高於一百二十日均線的家數。
    pub(crate) above_120_day_moving_average: i32,
    /// 股價不高於二百四十日均線的家數。
    pub(crate) below_240_day_moving_average: i32,
    /// 股價高於二百四十日均線的家數。
    pub(crate) above_240_day_moving_average: i32,
    /// 上漲家數。
    pub(crate) stocks_up: i32,
    /// 下跌家數。
    pub(crate) stocks_down: i32,
    /// 平盤家數。
    pub(crate) stocks_unchanged: i32,
    /// 最後更新時間，UTC ISO 8601。
    pub(crate) updated_at: Option<String>,
}

/// 市場廣度成功回應；形狀不因 `days` 改變。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct MarketBreadthResponse {
    /// `history[0]` 的日期。
    pub(crate) data_as_of: String,
    /// 最新一筆統計，永遠等於 `history[0]`。
    pub(crate) breadth: MarketBreadth,
    /// 由新到舊的交易日統計序列。
    pub(crate) history: Vec<MarketBreadth>,
}

/// 殖利率排行中的單一股票。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct DividendYieldRank {
    /// 一起始的名次。
    pub(crate) rank: u32,
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 市場編號（上市 2、上櫃 4）。
    pub(crate) market_id: i32,
    /// 產業分類編號。
    pub(crate) industry_id: i32,
    /// 排行資料日期，格式 `YYYY-MM-DD`。
    pub(crate) date: String,
    /// 計算殖利率所用收盤價。
    pub(crate) closing_price: Option<f64>,
    /// 計算殖利率所用年度股利。
    pub(crate) dividend: Option<f64>,
    /// 殖利率百分比。
    pub(crate) dividend_yield_percent: Option<f64>,
}

/// 殖利率排行成功回應（§3.4 envelope）。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct DividendYieldRankingResponse {
    /// 實際排行資料日；整張表無可用日期時不會產生此 response。
    pub(crate) data_as_of: String,
    /// 依殖利率由高到低、同值股票代號由小到大排序的股票。
    pub(crate) stocks: Vec<DividendYieldRank>,
}

/// 台股大盤指數（TAIEX）單一交易日的資料點（§4.8）。
///
/// 對應 `index` 表一列（`category = 'TAIEX'`）。數值欄位沿用 §3.1 規則：
/// `NUMERIC` 無法安全轉 `f64` 時輸出 `null`，資料庫中本來就是 `0` 的值
/// 維持 `0`，不推斷成缺值。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct MarketIndexPoint {
    /// 指數日期，格式 `YYYY-MM-DD`。
    pub(crate) date: String,
    /// 收盤指數（點）。
    pub(crate) index: Option<f64>,
    /// 漲跌點數；下跌為負值。
    pub(crate) change: Option<f64>,
    /// 成交金額（元）。
    pub(crate) trade_value: Option<f64>,
    /// 成交筆數。
    pub(crate) transaction: Option<f64>,
    /// 成交股數。
    pub(crate) trading_volume: Option<f64>,
}

/// 大盤指數歷史的成功回應（§3.4 envelope）。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct MarketIndexHistoryResponse {
    /// 實際回傳資料中最新一筆的日期（`YYYY-MM-DD`）；空清單時為 `null`。
    pub(crate) data_as_of: Option<String>,
    /// 指數資料點，依日期由新到舊。
    pub(crate) points: Vec<MarketIndexPoint>,
}

/// 股利行事曆中的單一事件（§4.9）。
///
/// 同一筆股利公告若有多個日期落在查詢區間，會展開成多筆事件（每筆一個
/// `event_type`）。日期欄位在資料庫是字串且含 `-`、`尚未公布` 等無效標記，
/// 只有合法 `YYYY-MM-DD` 的日期才會產生事件。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct DividendCalendarEvent {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 事件類型：`ex_dividend`、`ex_rights`、`cash_payable` 或 `stock_payable`。
    pub(crate) event_type: String,
    /// 事件日期，格式 `YYYY-MM-DD`。
    pub(crate) event_date: String,
    /// 股利所屬年度（西元）；DB 欄位 `year_of_dividend`。
    pub(crate) dividend_year: i32,
    /// 期間標記：`A`（年度）、`H1`／`H2`（半年度）或 `Q1`–`Q4`（§3.5）。
    pub(crate) quarter: String,
    /// 現金股利合計（元）。
    pub(crate) cash_dividend: Option<f64>,
    /// 股票股利合計（元）。
    pub(crate) stock_dividend: Option<f64>,
    /// 股利合計（元）；DB 欄位 `sum`。
    pub(crate) total_dividend: Option<f64>,
}

/// 股利行事曆的成功回應（§3.4 envelope）。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct DividendCalendarResponse {
    /// 混合事件沒有單一統計日期，固定為 `null`（各事件日期在每筆事件內）。
    pub(crate) data_as_of: Option<String>,
    /// 依 `event_date ASC`、同日 `stock_symbol ASC` 排序的事件。
    pub(crate) events: Vec<DividendCalendarEvent>,
}

/// QFII 持股排行中的單一股票（§4.10）。
///
/// 「QFII」指全體外資及陸資；數字來自 `stocks` 表的**當前快照**（每日
/// 22:00 UTC 排程更新），沒有歷史序列，不可用來回答增減持趨勢問題。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct QfiiHolding {
    /// 一起始的名次。
    pub(crate) rank: u32,
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 市場編號（上市 2、上櫃 4）。
    pub(crate) market_id: i32,
    /// 產業分類編號。
    pub(crate) industry_id: i32,
    /// 全體外資及陸資持有股數。
    pub(crate) qfii_shares_held: i64,
    /// 全體外資及陸資持股比率（%）。
    pub(crate) qfii_share_holding_percentage: Option<f64>,
    /// 發行股數。
    pub(crate) issued_share: i64,
}

/// QFII 持股排行的成功回應（§3.4 envelope）。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct QfiiHoldingRankingResponse {
    /// `stocks` 表快照沒有列級更新日期，不可偽造，固定為 `null`。
    pub(crate) data_as_of: Option<String>,
    /// 依指定指標由高到低、同值股票代號由小到大排序的股票。
    pub(crate) stocks: Vec<QfiiHolding>,
}

/// 市場廣度 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct MarketBreadthParams {
    /// 市場：`all`（預設）、`twse` 或 `tpex`。
    #[param(value_type = MarketParamValue, inline, default = "all")]
    pub(crate) market: Option<String>,
    /// 查詢截止日，格式 `YYYY-MM-DD`；未提供時取最新資料。
    pub(crate) date: Option<String>,
    /// 最近有資料的交易日筆數，預設 1，範圍 1–60。
    #[param(minimum = 1, maximum = 60, default = 1)]
    pub(crate) days: Option<u8>,
}

/// 殖利率排行 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct DividendYieldRankingParams {
    /// 查詢截止日，格式 `YYYY-MM-DD`；未提供時取最新資料。
    pub(crate) date: Option<String>,
    /// 市場：`all`（預設）、`twse` 或 `tpex`。
    #[param(value_type = MarketParamValue, inline, default = "all")]
    pub(crate) market: Option<String>,
    /// 可選的正整數產業分類編號。
    #[param(minimum = 1)]
    pub(crate) industry_id: Option<i32>,
    /// 最多回傳筆數，預設 20，範圍 1–50。
    #[param(minimum = 1, maximum = 50, default = 20)]
    pub(crate) limit: Option<u8>,
}

/// 大盤指數歷史 endpoint 的 query string（§4.8）。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct MarketIndexHistoryParams {
    /// 起始日期，格式 `YYYY-MM-DD`；未提供時不限制起點。
    pub(crate) from: Option<String>,
    /// 結束日期，格式 `YYYY-MM-DD`；未提供時不限制終點。
    pub(crate) to: Option<String>,
    /// 最多回傳筆數，預設 30，範圍 1–365（與歷史日線慣例一致）。
    #[param(minimum = 1, maximum = 365, default = 30)]
    pub(crate) limit: Option<u16>,
}

/// 股利行事曆 endpoint 的 query string（§4.9）。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct DividendCalendarParams {
    /// 起始日期，格式 `YYYY-MM-DD`；未提供時預設查詢當日（台北時區）。
    pub(crate) from: Option<String>,
    /// 結束日期，格式 `YYYY-MM-DD`；未提供時預設 `from + 30` 天。
    /// `to - from` 不可超過 92 天（一季），避免全表匯出。
    pub(crate) to: Option<String>,
    /// 事件類型：`all`（預設）、`ex_dividend`、`ex_rights`、`cash_payable`
    /// 或 `stock_payable`。
    #[param(value_type = CalendarEventTypeValue, inline, default = "all")]
    pub(crate) event_type: Option<String>,
    /// 最多回傳筆數，預設 50，範圍 1–200。
    #[param(minimum = 1, maximum = 200, default = 50)]
    pub(crate) limit: Option<u16>,
}

/// QFII 持股排行 endpoint 的 query string（§4.10）。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct QfiiHoldingRankingParams {
    /// 市場：`all`（預設）、`twse` 或 `tpex`。
    #[param(value_type = MarketParamValue, inline, default = "all")]
    pub(crate) market: Option<String>,
    /// 可選的正整數產業分類編號。
    #[param(minimum = 1)]
    pub(crate) industry_id: Option<i32>,
    /// 排序欄位：`percentage`（預設，持股比例）或 `shares`（持股股數）；
    /// 一律由高到低。
    #[param(value_type = QfiiSortValue, inline, default = "percentage")]
    pub(crate) sort_by: Option<String>,
    /// 最多回傳筆數，預設 20，範圍 1–50。
    #[param(minimum = 1, maximum = 50, default = 20)]
    pub(crate) limit: Option<u8>,
}
