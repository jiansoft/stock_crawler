//! 每日 CAGR（`/market/cagr-ranking`）的 request、response 與 OpenAPI
//! schema（M4）。
//!
//! 所有金額與比率一律序列化為字串：後端是 `rust_decimal`，轉成 JSON number
//! 會經過 f64 而掉精度；`principal` 與各種計數是整數，維持 number。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// OpenAPI 文件使用的 CAGR 統計期間。
///
/// 值與 `CagrPeriod::code()` 一一對應；runtime 仍以 `String` 解析，
/// 讓不合法的值得到明確的 422 訊息而非 serde 的泛用錯誤。
#[derive(ToSchema)]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema。
enum CagrPeriodParamValue {
    /// 3 個月。
    M3,
    /// 6 個月。
    M6,
    /// 1 年。
    Y1,
    /// 1 年 6 個月。
    Y1H,
    /// 2 年。
    Y2,
    /// 3 年。
    Y3,
    /// 5 年。
    Y5,
    /// 7 年。
    Y7,
    /// 10 年。
    Y10,
}

/// OpenAPI 文件使用的 CAGR 報酬口徑。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema。
enum CagrMetricParamValue {
    /// 口徑 A：純價格報酬（長期間不提供）。
    Price,
    /// 口徑 B：含息不再投入，主指標。
    Total,
    /// 口徑 C：含息再投入。
    Reinvested,
}

/// OpenAPI 文件使用的 CAGR 排行排序鍵。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema。
enum CagrSortParamValue {
    /// 依年化報酬率排序。
    Cagr,
    /// 依區間總報酬率排序。
    TotalReturn,
}

/// 每日 CAGR 排行 endpoint 的 query string。
///
/// 參數名與前端 `stock_go` 送出的完全一致，勿更名；不合法的值一律在
/// handler 內以固定訊息回 422，而非交給 serde 產生泛用錯誤。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct CagrRankingParams {
    /// 統計期間：`M3`、`M6`、`Y1`（預設）、`Y1H`、`Y2`、`Y3`、`Y5`、`Y7` 或 `Y10`。
    #[param(value_type = CagrPeriodParamValue, inline, default = "Y1")]
    pub(crate) period: Option<String>,
    /// 報酬口徑：`price`、`total`（預設）或 `reinvested`。
    ///
    /// `Y5`／`Y10` 不接受 `price`——近十年每年皆有 134–216 檔股票配股，
    /// 長期間忽略配股的誤差顯著，因此回 422 而非默默給出低估的數字。
    #[param(value_type = CagrMetricParamValue, inline, default = "total")]
    pub(crate) metric: Option<String>,
    /// 排序鍵：`cagr` 或 `total_return`；未指定時依期間長度自動選擇
    /// （12 個月以上用 `cagr`，短期間用 `total_return`）。
    #[param(value_type = CagrSortParamValue, inline)]
    pub(crate) sort: Option<String>,
    /// 市場：`all`（預設）、`twse`、`tpex`，或直接給市場編號。
    #[param(default = "all")]
    pub(crate) market: Option<String>,
    /// 可選的正整數產業分類編號。
    #[param(minimum = 1)]
    pub(crate) stock_industry_id: Option<i32>,
    /// 代號或名稱關鍵字（模糊比對），長度 1–50。
    pub(crate) keyword: Option<String>,
    /// 是否一併回傳資料不足的項目，預設 `true`。
    ///
    /// 預設帶回是刻意的：「查得到但算不出來」與「查不到」是兩件不同的事。
    #[param(default = true)]
    pub(crate) include_incomplete: Option<bool>,
    /// 最多回傳筆數，預設 50，範圍 1–200。
    #[param(minimum = 1, maximum = 200, default = 50)]
    pub(crate) limit: Option<u16>,
    /// 起始位移，預設 0。
    #[param(minimum = 0, default = 0)]
    pub(crate) offset: Option<u32>,
    /// 計算基準日，格式 `YYYY-MM-DD`；未提供時取最新一個已完成計算的日期。
    pub(crate) date: Option<String>,
}

/// 個股 CAGR 全期間 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct CagrSymbolParams {
    /// 報酬口徑：`price`、`total`（預設）或 `reinvested`。
    #[param(value_type = CagrMetricParamValue, inline, default = "total")]
    pub(crate) metric: Option<String>,
    /// 計算基準日，格式 `YYYY-MM-DD`；未提供時取最新一個已完成計算的日期。
    pub(crate) date: Option<String>,
}

/// CAGR 樣本涵蓋統計。
///
/// `coverage_ratio` 的分母是母體檔數（`universe`），用來揭露長期間的樣本
/// 缺口；`summary.positive_ratio` 的分母則是可算檔數（`counted`），兩者
/// 分母不同，混用會安靜地得到偏低的數字。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct CagrCoverageInfo {
    /// 母體檔數（該基準日與期間寫入的全部資料列）。
    pub(crate) universe: i64,
    /// 實際可算檔數。
    pub(crate) counted: i64,
    /// 樣本涵蓋率＝`counted / universe`，字串固定四位小數。
    pub(crate) coverage_ratio: String,
    /// 資料不足檔數。
    pub(crate) incomplete: i64,
    /// 標記為疑似減資／分割的檔數。
    pub(crate) anomaly_flagged: i64,
    /// 是否需要顯示存活者偏誤揭露條（Y5／Y10 為 `true`）。
    pub(crate) survivorship_note: bool,
}

/// CAGR 排行的正報酬摘要。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct CagrSummary {
    /// 可算檔數中報酬為正的檔數。
    pub(crate) positive: i64,
    /// 正報酬比例＝`positive / counted`，字串固定四位小數。
    pub(crate) positive_ratio: String,
}

/// CAGR 排行中的單一股票。
///
/// 所有金額與比率一律序列化為字串：後端是 `rust_decimal`，轉成 JSON number
/// 會經過 f64 而掉精度。資料不足的項目仍會出現在清單中，只是各數值欄位為
/// `null`——前端據此渲染灰化列，而不是把它當成不存在的股票。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct CagrRankingItem {
    /// 全市場名次；資料不足者為 `null` 且不佔名次。
    ///
    /// 名次是該 `(date, period, metric)` 之下**未套用篩選**的完整市場排名，
    /// 因此篩選單一產業後可能看到 1、17、43 這種不連續的值——這是刻意的：
    /// 篩選後的「第 1 名」不是市場第 1 名，重新編號會誤導使用者。
    pub(crate) rank: Option<i64>,
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 產業分類編號。
    pub(crate) stock_industry_id: i32,
    /// 實際採用的期初交易日，格式 `YYYY-MM-DD`。
    pub(crate) base_date: Option<String>,
    /// 該股最早的報價日，格式 `YYYY-MM-DD`。
    pub(crate) first_quote_date: Option<String>,
    /// 期初日較名目期初目標日順延的天數；0 表完全對齊。
    pub(crate) shortfall_days: Option<i32>,
    /// 期初收盤價。
    pub(crate) base_price: Option<String>,
    /// 期末收盤價。
    pub(crate) end_price: Option<String>,
    /// 期末持有股數（含配股）。
    pub(crate) end_shares: Option<String>,
    /// 累積現金股利。
    pub(crate) cash_received: Option<String>,
    /// 期末總價值。
    pub(crate) end_value: Option<String>,
    /// 區間總報酬率（%）。
    pub(crate) total_return_pct: Option<String>,
    /// 年化報酬率（%）。
    pub(crate) cagr_pct: Option<String>,
    /// 期間內實際採計的除權息次數。
    pub(crate) dividend_events: i32,
    /// 期初資料是否齊全。
    pub(crate) data_complete: bool,
    /// 是否偵測到無對應除權息的異常跳動（疑似減資或分割）。
    pub(crate) has_anomaly: bool,
}

/// 個股 CAGR 的單一期間結果。
///
/// 欄位與 [`CagrRankingItem`] 相同，但多一個 `period`、沒有 `rank`——
/// 個股頁是「同一檔股票的八個期間」，名次在此沒有意義。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct CagrPeriodItem {
    /// 統計期間代碼。
    pub(crate) period: String,
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 產業分類編號。
    pub(crate) stock_industry_id: i32,
    /// 實際採用的期初交易日，格式 `YYYY-MM-DD`。
    pub(crate) base_date: Option<String>,
    /// 該股最早的報價日，格式 `YYYY-MM-DD`。
    pub(crate) first_quote_date: Option<String>,
    /// 期初日較名目期初目標日順延的天數。
    pub(crate) shortfall_days: Option<i32>,
    /// 期初收盤價。
    pub(crate) base_price: Option<String>,
    /// 期末收盤價。
    pub(crate) end_price: Option<String>,
    /// 期末持有股數（含配股）。
    pub(crate) end_shares: Option<String>,
    /// 累積現金股利。
    pub(crate) cash_received: Option<String>,
    /// 期末總價值。
    pub(crate) end_value: Option<String>,
    /// 區間總報酬率（%）。
    pub(crate) total_return_pct: Option<String>,
    /// 年化報酬率（%）。
    pub(crate) cagr_pct: Option<String>,
    /// 實際年數＝（期末交易日 − 期初交易日）／365。
    pub(crate) years: Option<String>,
    /// 期間內實際採計的除權息次數。
    pub(crate) dividend_events: i32,
    /// 期初資料是否齊全。
    pub(crate) data_complete: bool,
    /// 是否偵測到疑似減資或分割的異常跳動。
    pub(crate) has_anomaly: bool,
    /// 該期間是否需要顯示存活者偏誤揭露條。
    pub(crate) survivorship_note: bool,
}

/// CAGR 排行的成功回應。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct CagrRankingResponse {
    /// 統計期間代碼。
    pub(crate) period: String,
    /// 報酬口徑代碼。
    pub(crate) metric: String,
    /// 實際採用的排序鍵代碼。
    pub(crate) sort: String,
    /// 計算基準日，格式 `YYYY-MM-DD`。
    pub(crate) date: String,
    /// 本頁可算項目中最常見的期初交易日；全部資料不足時為 `null`。
    pub(crate) base_date: Option<String>,
    /// 對應 `base_date` 的實際年數，字串固定四位小數。
    pub(crate) years: Option<String>,
    /// 固定投入金額（元）。整數，維持 JSON number。
    pub(crate) principal: i64,
    /// 符合篩選條件的總筆數，供分頁計算。
    pub(crate) total: i64,
    /// 樣本涵蓋統計（不受畫面篩選影響）。
    pub(crate) coverage: CagrCoverageInfo,
    /// 正報酬摘要。
    pub(crate) summary: CagrSummary,
    /// 本頁項目；可算項目在前依排序鍵由高到低，資料不足者殿後。
    pub(crate) items: Vec<CagrRankingItem>,
}

/// 個股 CAGR 全期間的成功回應。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct CagrSymbolResponse {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: Option<String>,
    /// 報酬口徑代碼。
    pub(crate) metric: String,
    /// 計算基準日，格式 `YYYY-MM-DD`。
    pub(crate) date: String,
    /// 固定投入金額（元）。
    pub(crate) principal: i64,
    /// 全部八個期間，依期間長度由短至長；含資料不足者。
    pub(crate) items: Vec<CagrPeriodItem>,
}
