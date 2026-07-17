//! Data API 的 request、response 與 OpenAPI schema。
//!
//! 此檔案刻意只放 HTTP 契約型別，避免 SQLx 的資料庫列型別滲漏到 API；所有
//! 缺值欄位皆保留 `Option`，讓 serde 輸出 JSON `null` 而非猜測成零值。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// 股票基本資料。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct Stock {
    /// 系統內股票代號。
    pub(super) stock_symbol: String,
    /// 證券代號。
    pub(super) security_code: String,
    /// 股票名稱。
    pub(super) name: String,
    /// 交易所市場編號。
    pub(super) stock_exchange_market_id: i32,
    /// 產業分類編號。
    pub(super) stock_industry_id: i32,
    /// 是否暫停交易或下市。
    pub(super) suspend_listing: bool,
}

/// 最新單一交易日的日報價。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct DailyQuote {
    /// 交易日期，格式為 `YYYY-MM-DD`。
    pub(super) date: String,
    /// 開盤價。
    pub(super) opening_price: Option<f64>,
    /// 最高價。
    pub(super) highest_price: Option<f64>,
    /// 最低價。
    pub(super) lowest_price: Option<f64>,
    /// 收盤價。
    pub(super) closing_price: Option<f64>,
    /// 漲跌金額。
    pub(super) change: Option<f64>,
    /// 漲跌幅。
    pub(super) change_range: Option<f64>,
    /// 成交股數。
    pub(super) trading_volume: Option<f64>,
    /// 成交筆數。
    pub(super) transaction: Option<f64>,
    /// 成交金額。
    pub(super) trade_value: Option<f64>,
    /// 五日均線。
    pub(super) moving_average_5: Option<f64>,
    /// 十日均線。
    pub(super) moving_average_10: Option<f64>,
    /// 二十日均線。
    pub(super) moving_average_20: Option<f64>,
    /// 六十日均線。
    pub(super) moving_average_60: Option<f64>,
    /// 一百二十日均線。
    pub(super) moving_average_120: Option<f64>,
    /// 二百四十日均線。
    pub(super) moving_average_240: Option<f64>,
    /// 本益比。
    pub(super) price_earning_ratio: Option<f64>,
    /// 資料紀錄時間，UTC ISO 8601。
    pub(super) record_time: Option<String>,
    /// 最後更新時間，UTC ISO 8601。
    pub(super) updated_time: Option<String>,
}

/// 歷史日線資料。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct HistoricalQuote {
    /// 交易日期，格式為 `YYYY-MM-DD`。
    pub(super) date: String,
    /// 開盤價。
    pub(super) opening_price: Option<f64>,
    /// 最高價。
    pub(super) highest_price: Option<f64>,
    /// 最低價。
    pub(super) lowest_price: Option<f64>,
    /// 收盤價。
    pub(super) closing_price: Option<f64>,
    /// 漲跌金額。
    pub(super) change: Option<f64>,
    /// 漲跌幅。
    pub(super) change_range: Option<f64>,
    /// 成交股數。
    pub(super) trading_volume: Option<f64>,
    /// 成交筆數。
    pub(super) transaction: Option<f64>,
    /// 成交金額。
    pub(super) trade_value: Option<f64>,
    /// 五日均線。
    pub(super) moving_average_5: Option<f64>,
    /// 十日均線。
    pub(super) moving_average_10: Option<f64>,
    /// 二十日均線。
    pub(super) moving_average_20: Option<f64>,
    /// 六十日均線。
    pub(super) moving_average_60: Option<f64>,
    /// 本益比。
    pub(super) price_earning_ratio: Option<f64>,
    /// 股價淨值比。
    pub(super) price_to_book_ratio: Option<f64>,
    /// 資料紀錄時間，UTC ISO 8601。
    pub(super) record_time: Option<String>,
}

/// 系統收錄範圍內的歷史高低點。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct QuoteHistoryRecord {
    /// 歷史最高價。
    pub(super) maximum_price: Option<f64>,
    /// 最高價日期。
    pub(super) maximum_price_date_on: Option<String>,
    /// 歷史最低價。
    pub(super) minimum_price: Option<f64>,
    /// 最低價日期。
    pub(super) minimum_price_date_on: Option<String>,
    /// 歷史最高股價淨值比。
    pub(super) maximum_price_to_book_ratio: Option<f64>,
    /// 最高股價淨值比日期。
    pub(super) maximum_price_to_book_ratio_date_on: Option<String>,
    /// 歷史最低股價淨值比。
    pub(super) minimum_price_to_book_ratio: Option<f64>,
    /// 最低股價淨值比日期。
    pub(super) minimum_price_to_book_ratio_date_on: Option<String>,
}

/// 股票完整資料。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct StockProfile {
    /// 股票基本資料。
    pub(super) stock: Stock,
    /// 最新日報價；沒有日報價時為 `null`。
    pub(super) quote: Option<DailyQuote>,
    /// 近一季 EPS。
    pub(super) last_one_eps: Option<f64>,
    /// 近四季 EPS 合計。
    pub(super) last_four_eps: Option<f64>,
    /// 每股淨值。
    pub(super) net_asset_value_per_share: Option<f64>,
    /// 股東權益報酬率。
    pub(super) return_on_equity: Option<f64>,
    /// 權值。
    pub(super) weight: Option<f64>,
    /// 發行股數。
    pub(super) issued_share: Option<f64>,
    /// 歷史高低點；沒有紀錄時為 `null`。
    pub(super) history: Option<QuoteHistoryRecord>,
}

/// 搜尋股票的成功回應。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct SearchResponse {
    /// 搜尋結果。
    pub(super) stocks: Vec<Stock>,
}
/// 最新日報價的成功回應。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct LatestQuoteResponse {
    /// 股票基本資料。
    pub(super) stock: Stock,
    /// 最新日報價；沒有資料時為 null。
    pub(super) quote: Option<DailyQuote>,
}
/// 歷史日線的成功回應。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct PriceHistoryResponse {
    /// 符合範圍的歷史日線。
    pub(super) quotes: Vec<HistoricalQuote>,
}
/// 統一錯誤回應。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct ErrorBody {
    /// 不含內部實作細節的錯誤訊息。
    pub(super) error: String,
}
/// 健康檢查成功回應。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct HealthResponse {
    /// 服務狀態。
    pub(super) status: &'static str,
}

/// 第三方網站採集的近即時報價快照。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct RealtimeSnapshotResponse {
    /// 股票代號。
    pub(super) stock_symbol: String,
    /// 股票名稱。
    pub(super) name: String,
    /// 成交價。
    pub(super) price: Option<f64>,
    /// 漲跌。
    pub(super) change: Option<f64>,
    /// 漲跌幅。
    pub(super) change_range: Option<f64>,
    /// 開盤價。
    pub(super) open: Option<f64>,
    /// 最高價。
    pub(super) high: Option<f64>,
    /// 最低價。
    pub(super) low: Option<f64>,
    /// 昨收價。
    pub(super) last_close: Option<f64>,
    /// 成交量，單位為張。
    pub(super) volume_lots: Option<f64>,
    /// 採集來源站點。
    pub(super) source_site: String,
    /// 快照寫入快取的 UTC ISO 8601 時間。
    pub(super) updated_at: String,
}

/// 單月營收資料。
///
/// 對應資料表 `"Revenue"` 一列；`month` 由資料庫的 `YYYYMM` 整數（例如
/// `202606`）轉成 `YYYY-MM` 字串，內部編碼不對外暴露。金額與百分比欄位
/// 沿用 §3.1 規則：`NUMERIC` 無法安全轉 `f64` 時輸出 `null`，資料庫中
/// 本來就是 `0` 的值維持 `0`，不推斷成缺值。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct MonthlyRevenue {
    /// 營收月份，格式 `YYYY-MM`。
    pub(super) month: String,
    /// 當月營收（仟元）。
    pub(super) monthly_revenue: Option<f64>,
    /// 上月營收（仟元）。
    pub(super) last_month_revenue: Option<f64>,
    /// 去年同月營收（仟元）。
    pub(super) last_year_same_month_revenue: Option<f64>,
    /// 當年度累計營收（仟元）。
    pub(super) monthly_accumulated_revenue: Option<f64>,
    /// 去年同期累計營收（仟元）。
    pub(super) last_year_monthly_accumulated_revenue: Option<f64>,
    /// 月增率（%）。
    pub(super) month_over_month_percent: Option<f64>,
    /// 年增率（%）。
    pub(super) year_over_year_percent: Option<f64>,
    /// 累計年增率（%）。
    pub(super) accumulated_year_over_year_percent: Option<f64>,
    /// 當月均價（元）。
    pub(super) average_price: Option<f64>,
    /// 當月最低價（元）。
    pub(super) lowest_price: Option<f64>,
    /// 當月最高價（元）。
    pub(super) highest_price: Option<f64>,
}

/// 月營收歷史的成功回應（§3.4 envelope）。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct MonthlyRevenueResponse {
    /// 股票代號。
    pub(super) stock_symbol: String,
    /// 實際回傳資料中最新一期的月份（`YYYY-MM`）；空清單時為 `null`。
    pub(super) data_as_of: Option<String>,
    /// 月營收清單，依月份由新到舊。
    pub(super) revenues: Vec<MonthlyRevenue>,
}

/// 單期財務報表（獲利能力與每股數據）。
///
/// `quarter` 依 §3.5 對映：資料庫以空字串代表年度資料，API 契約統一輸出
/// `A`；季度資料維持 `Q1`–`Q4`。百分比欄位（毛利率等）在資料庫已是
/// 百分比數值，不再另行換算。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct FinancialStatement {
    /// 年度（西元）。
    pub(super) year: i64,
    /// 期間標記：`A`（年度）或 `Q1`–`Q4`。
    pub(super) quarter: String,
    /// 營業毛利率（%）；DB 欄位 `gross_profit`。
    pub(super) gross_profit_margin: Option<f64>,
    /// 營業利益率（%）。
    pub(super) operating_profit_margin: Option<f64>,
    /// 稅前淨利率（%）；DB 欄位 `pre_tax_income`。
    pub(super) pre_tax_income_margin: Option<f64>,
    /// 稅後淨利率（%）；DB 欄位 `net_income`。
    pub(super) net_income_margin: Option<f64>,
    /// 每股淨值（元）。
    pub(super) net_asset_value_per_share: Option<f64>,
    /// 每股營收（元）。
    pub(super) sales_per_share: Option<f64>,
    /// 每股稅後盈餘 EPS（元）。
    pub(super) earnings_per_share: Option<f64>,
    /// 每股稅前淨利（元）；DB 欄位 `profit_before_tax`。
    pub(super) profit_before_tax_per_share: Option<f64>,
    /// 股東權益報酬率 ROE（%）。
    pub(super) return_on_equity: Option<f64>,
    /// 資產報酬率 ROA（%）。
    pub(super) return_on_assets: Option<f64>,
    /// 最後更新時間，UTC ISO 8601；DB 欄位 `updated_time`。
    pub(super) updated_at: Option<String>,
}

/// 財報歷史的成功回應（§3.4 envelope）。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct FinancialStatementHistoryResponse {
    /// 股票代號。
    pub(super) stock_symbol: String,
    /// 實際回傳資料中最新一期的期間標記（如 `2026-Q1`、`2025-A`）；空清單時為 `null`。
    pub(super) data_as_of: Option<String>,
    /// 財報清單，依 §3.4 期間順序由新到舊。
    pub(super) statements: Vec<FinancialStatement>,
}

/// 單筆股利發放資料。
///
/// 年度欄位語意（§4.3 對照表）：`paid_year` 是「發放年度」（DB `year`），
/// `dividend_year` 是「股利所屬年度」（DB `year_of_dividend`）；兩者常差
/// 一年，年份篩選一律依 `dividend_year`。日期欄位在資料庫是字串，只有可
/// 解析為合法 `YYYY-MM-DD` 的值才輸出，`-`、`尚未公布` 等標記一律 `null`。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct Dividend {
    /// 發放年度（西元）；DB 欄位 `year`。
    pub(super) paid_year: i32,
    /// 股利所屬年度（西元）；DB 欄位 `year_of_dividend`。
    pub(super) dividend_year: i32,
    /// 期間標記：`A`（年度）、`H1`／`H2`（半年度）或 `Q1`–`Q4`。
    pub(super) quarter: String,
    /// 現金股利合計（元）。
    pub(super) cash_dividend: Option<f64>,
    /// 股票股利合計（元）。
    pub(super) stock_dividend: Option<f64>,
    /// 股利合計（元）；DB 欄位 `sum`。
    pub(super) total_dividend: Option<f64>,
    /// 盈餘配息（元）。
    pub(super) earnings_cash_dividend: Option<f64>,
    /// 公積配息（元）。
    pub(super) capital_reserve_cash_dividend: Option<f64>,
    /// 盈餘配股（元）。
    pub(super) earnings_stock_dividend: Option<f64>,
    /// 公積配股（元）。
    pub(super) capital_reserve_stock_dividend: Option<f64>,
    /// 盈餘分配率＿配息（%）；DB 欄位 `payout_ratio_cash`。
    pub(super) cash_payout_ratio: Option<f64>,
    /// 盈餘分配率＿配股（%）；DB 欄位 `payout_ratio_stock`。
    pub(super) stock_payout_ratio: Option<f64>,
    /// 盈餘分配率合計（%）；DB 欄位 `payout_ratio`。
    pub(super) total_payout_ratio: Option<f64>,
    /// 除息日；DB 欄位 `"ex-dividend_date1"`，無效標記為 `null`。
    pub(super) ex_dividend_date: Option<String>,
    /// 除權日；DB 欄位 `"ex-dividend_date2"`，無效標記為 `null`。
    pub(super) ex_rights_date: Option<String>,
    /// 現金股利發放日；DB 欄位 `payable_date1`，無效標記為 `null`。
    pub(super) cash_payable_date: Option<String>,
    /// 股票股利發放日；DB 欄位 `payable_date2`，無效標記為 `null`。
    pub(super) stock_payable_date: Option<String>,
    /// 最後更新時間，UTC ISO 8601；DB 欄位 `updated_time`。
    pub(super) updated_at: Option<String>,
}

/// 股利歷史的成功回應（§3.4 envelope）。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct DividendHistoryResponse {
    /// 股票代號。
    pub(super) stock_symbol: String,
    /// 實際回傳資料中最新一期的期間標記（如 `2025-A`、`2025-Q4`）；空清單時為 `null`。
    pub(super) data_as_of: Option<String>,
    /// 股利清單，依 §3.4 期間順序由新到舊。
    pub(super) dividends: Vec<Dividend>,
}

/// 搜尋 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(super) struct SearchParams {
    /// 搜尋字串，長度 1 至 100。
    pub(super) query: String,
    /// 最多回傳筆數，預設 10。
    pub(super) limit: Option<u8>,
}
/// 歷史日線 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(super) struct HistoryParams {
    /// 起始日期，格式 YYYY-MM-DD。
    pub(super) from: Option<String>,
    /// 結束日期，格式 YYYY-MM-DD。
    pub(super) to: Option<String>,
    /// 最多回傳筆數，預設 100。
    pub(super) limit: Option<u16>,
}
/// 月營收歷史 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(super) struct RevenueHistoryParams {
    /// 起始月份，格式 YYYY-MM。
    pub(super) from: Option<String>,
    /// 結束月份，格式 YYYY-MM。
    pub(super) to: Option<String>,
    /// 最多回傳筆數，預設 24，範圍 1–120。
    pub(super) limit: Option<u16>,
}
/// 財報歷史 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(super) struct StatementHistoryParams {
    /// 期間類型：`quarterly`（預設）、`annual` 或 `all`。
    pub(super) period_type: Option<String>,
    /// 最多回傳筆數，預設 12，範圍 1–40。
    pub(super) limit: Option<u16>,
}
/// 股利歷史 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(super) struct DividendHistoryParams {
    /// 起始年度（股利所屬年度，西元）。
    pub(super) from_year: Option<i32>,
    /// 結束年度（股利所屬年度，西元）。
    pub(super) to_year: Option<i32>,
    /// 最多回傳筆數，預設 20，範圍 1–80。
    pub(super) limit: Option<u16>,
}
