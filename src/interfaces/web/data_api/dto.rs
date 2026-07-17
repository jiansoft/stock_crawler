//! Data API 的 request、response 與 OpenAPI schema。
//!
//! 此檔案刻意只放 HTTP 契約型別，避免 SQLx 的資料庫列型別滲漏到 API；所有
//! 缺值欄位皆保留 `Option`，讓 serde 輸出 JSON `null` 而非猜測成零值。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// OpenAPI 文件使用的三種查詢市場值。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema；runtime 仍以 String 回傳精確 422。
enum MarketParamValue {
    /// 上市與上櫃合併。
    All,
    /// 僅上市。
    Twse,
    /// 僅上櫃。
    Tpex,
}

/// OpenAPI 文件使用的財報期間類型。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema。
enum StatementPeriodTypeValue {
    /// 僅季度資料。
    Quarterly,
    /// 僅年度資料。
    Annual,
    /// 年度與季度資料。
    All,
}

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

/// 單一交易日的個股估值計算結果。
///
/// 所有價格皆是 `estimate` 已完成的歷史模型計算值，不是目標價或買賣建議；
/// `valuation_band` 僅描述收盤價落在加權便宜／合理／昂貴價的哪一段。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct StockValuation {
    /// 股票代號。
    pub(super) stock_symbol: String,
    /// 估值資料日期，格式 `YYYY-MM-DD`。
    pub(super) date: String,
    /// 當日收盤價。
    pub(super) closing_price: Option<f64>,
    /// 收盤價相對加權便宜價的百分比。
    pub(super) percentage: Option<f64>,
    /// 參與模型計算的歷史年度數。
    pub(super) year_count: i32,
    /// 加權便宜價。
    pub(super) cheap: Option<f64>,
    /// 加權合理價。
    pub(super) fair: Option<f64>,
    /// 加權昂貴價。
    pub(super) expensive: Option<f64>,
    /// 歷史價格法便宜價。
    pub(super) price_cheap: Option<f64>,
    /// 歷史價格法合理價。
    pub(super) price_fair: Option<f64>,
    /// 歷史價格法昂貴價。
    pub(super) price_expensive: Option<f64>,
    /// 股利法便宜價。
    pub(super) dividend_cheap: Option<f64>,
    /// 股利法合理價。
    pub(super) dividend_fair: Option<f64>,
    /// 股利法昂貴價。
    pub(super) dividend_expensive: Option<f64>,
    /// EPS 法便宜價。
    pub(super) eps_cheap: Option<f64>,
    /// EPS 法合理價。
    pub(super) eps_fair: Option<f64>,
    /// EPS 法昂貴價。
    pub(super) eps_expensive: Option<f64>,
    /// PBR 法便宜價。
    pub(super) pbr_cheap: Option<f64>,
    /// PBR 法合理價。
    pub(super) pbr_fair: Option<f64>,
    /// PBR 法昂貴價。
    pub(super) pbr_expensive: Option<f64>,
    /// PER 法便宜價。
    pub(super) per_cheap: Option<f64>,
    /// PER 法合理價。
    pub(super) per_fair: Option<f64>,
    /// PER 法昂貴價。
    pub(super) per_expensive: Option<f64>,
    /// 收盤價相對加權估值區間的固定分類。
    pub(super) valuation_band: String,
}

/// 個股估值成功回應（§3.4 envelope）。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct StockValuationResponse {
    /// 股票代號。
    pub(super) stock_symbol: String,
    /// 實際回傳估值的日期；無資料時為 `null`。
    pub(super) data_as_of: Option<String>,
    /// 最近有效估值；31 天視窗內無資料時為 `null`。
    pub(super) valuation: Option<StockValuation>,
}

/// 單一交易日的市場廣度統計。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(super) struct MarketBreadth {
    /// 統計日期，格式 `YYYY-MM-DD`。
    pub(super) date: String,
    /// 市場名稱：`all`、`twse` 或 `tpex`。
    pub(super) market: String,
    /// 股價不高於便宜價的家數。
    pub(super) undervalued: i32,
    /// 股價介於便宜價與合理價的家數。
    pub(super) fair_valued: i32,
    /// 股價介於合理價與昂貴價的家數。
    pub(super) overvalued: i32,
    /// 股價高於昂貴價的家數。
    pub(super) highly_overvalued: i32,
    /// 股價不高於五日均線的家數。
    pub(super) below_5_day_moving_average: i32,
    /// 股價高於五日均線的家數。
    pub(super) above_5_day_moving_average: i32,
    /// 股價不高於二十日均線的家數。
    pub(super) below_20_day_moving_average: i32,
    /// 股價高於二十日均線的家數。
    pub(super) above_20_day_moving_average: i32,
    /// 股價不高於六十日均線的家數。
    pub(super) below_60_day_moving_average: i32,
    /// 股價高於六十日均線的家數。
    pub(super) above_60_day_moving_average: i32,
    /// 股價不高於一百二十日均線的家數。
    pub(super) below_120_day_moving_average: i32,
    /// 股價高於一百二十日均線的家數。
    pub(super) above_120_day_moving_average: i32,
    /// 股價不高於二百四十日均線的家數。
    pub(super) below_240_day_moving_average: i32,
    /// 股價高於二百四十日均線的家數。
    pub(super) above_240_day_moving_average: i32,
    /// 上漲家數。
    pub(super) stocks_up: i32,
    /// 下跌家數。
    pub(super) stocks_down: i32,
    /// 平盤家數。
    pub(super) stocks_unchanged: i32,
    /// 最後更新時間，UTC ISO 8601。
    pub(super) updated_at: Option<String>,
}

/// 市場廣度成功回應；形狀不因 `days` 改變。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct MarketBreadthResponse {
    /// `history[0]` 的日期。
    pub(super) data_as_of: String,
    /// 最新一筆統計，永遠等於 `history[0]`。
    pub(super) breadth: MarketBreadth,
    /// 由新到舊的交易日統計序列。
    pub(super) history: Vec<MarketBreadth>,
}

/// 殖利率排行中的單一股票。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct DividendYieldRank {
    /// 一起始的名次。
    pub(super) rank: u32,
    /// 股票代號。
    pub(super) stock_symbol: String,
    /// 股票名稱。
    pub(super) name: String,
    /// 市場編號（上市 2、上櫃 4）。
    pub(super) market_id: i32,
    /// 產業分類編號。
    pub(super) industry_id: i32,
    /// 排行資料日期，格式 `YYYY-MM-DD`。
    pub(super) date: String,
    /// 計算殖利率所用收盤價。
    pub(super) closing_price: Option<f64>,
    /// 計算殖利率所用年度股利。
    pub(super) dividend: Option<f64>,
    /// 殖利率百分比。
    pub(super) dividend_yield_percent: Option<f64>,
}

/// 殖利率排行成功回應（§3.4 envelope）。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct DividendYieldRankingResponse {
    /// 實際排行資料日；整張表無可用日期時不會產生此 response。
    pub(super) data_as_of: String,
    /// 依殖利率由高到低、同值股票代號由小到大排序的股票。
    pub(super) stocks: Vec<DividendYieldRank>,
}

/// 條件選股結果中的單一股票。
///
/// 四組來源期間即使已超過新鮮度上限仍會保留，方便呼叫端判斷資料為何被
/// 轉成 `null`；過期的指標本身不參與篩選，也不會被誤當成目前數值。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct ScreenedStock {
    /// 股票代號。
    pub(super) stock_symbol: String,
    /// 股票名稱。
    pub(super) name: String,
    /// 市場編號（上市 2、上櫃 4）。
    pub(super) market_id: i32,
    /// 產業分類編號。
    pub(super) industry_id: i32,
    /// 最新且仍在三個月內的營收年增率；缺值或過期時為 `null`。
    pub(super) revenue_yoy_percent: Option<f64>,
    /// 最新且仍在兩季內的季度 EPS；缺值或過期時為 `null`。
    pub(super) earnings_per_share: Option<f64>,
    /// 最新且仍在兩季內的季度 ROE；缺值或過期時為 `null`。
    pub(super) return_on_equity: Option<f64>,
    /// 最新且仍在 31 天內的殖利率百分比；缺值或過期時為 `null`。
    pub(super) dividend_yield_percent: Option<f64>,
    /// 最新且仍在 31 天內的估值區間；缺值或過期時為 `null`。
    pub(super) valuation_band: Option<String>,
    /// 最新且仍在 31 天內的估值百分比；缺值或過期時為 `null`。
    pub(super) valuation_percentage: Option<f64>,
    /// 該股票最新營收月份，格式 `YYYY-MM`；過期時仍保留。
    pub(super) revenue_month: Option<String>,
    /// 該股票最新季度財報期間，格式 `YYYY-Q1`～`YYYY-Q4`；過期時仍保留。
    pub(super) financial_period: Option<String>,
    /// 該股票最新估值日期，格式 `YYYY-MM-DD`；過期時仍保留。
    pub(super) valuation_date: Option<String>,
    /// 該股票最新殖利率日期，格式 `YYYY-MM-DD`；過期時仍保留。
    pub(super) yield_date: Option<String>,
}

/// 條件選股成功回應（§3.4 envelope）。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct StockScreeningResponse {
    /// 混合資料來源沒有單一正確日期，因此固定為 `null`。
    pub(super) data_as_of: Option<String>,
    /// 通過所有固定條件的股票，最多五十筆。
    pub(super) stocks: Vec<ScreenedStock>,
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
    #[param(minimum = 1, maximum = 120, default = 24)]
    pub(super) limit: Option<u16>,
}
/// 財報歷史 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(super) struct StatementHistoryParams {
    /// 期間類型：`quarterly`（預設）、`annual` 或 `all`。
    #[param(value_type = StatementPeriodTypeValue, inline, default = "quarterly")]
    pub(super) period_type: Option<String>,
    /// 最多回傳筆數，預設 12，範圍 1–40。
    #[param(minimum = 1, maximum = 40, default = 12)]
    pub(super) limit: Option<u16>,
}
/// 股利歷史 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(super) struct DividendHistoryParams {
    /// 起始年度（股利所屬年度，西元）。
    #[param(minimum = 1990)]
    pub(super) from_year: Option<i32>,
    /// 結束年度（股利所屬年度，西元）。
    #[param(minimum = 1990)]
    pub(super) to_year: Option<i32>,
    /// 最多回傳筆數，預設 20，範圍 1–80。
    #[param(minimum = 1, maximum = 80, default = 20)]
    pub(super) limit: Option<u16>,
}

/// 個股估值 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(super) struct ValuationParams {
    /// 查詢截止日，格式 `YYYY-MM-DD`；未提供時取最新資料。
    pub(super) date: Option<String>,
}

/// 市場廣度 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(super) struct MarketBreadthParams {
    /// 市場：`all`（預設）、`twse` 或 `tpex`。
    #[param(value_type = MarketParamValue, inline, default = "all")]
    pub(super) market: Option<String>,
    /// 查詢截止日，格式 `YYYY-MM-DD`；未提供時取最新資料。
    pub(super) date: Option<String>,
    /// 最近有資料的交易日筆數，預設 1，範圍 1–60。
    #[param(minimum = 1, maximum = 60, default = 1)]
    pub(super) days: Option<u8>,
}

/// 殖利率排行 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(super) struct DividendYieldRankingParams {
    /// 查詢截止日，格式 `YYYY-MM-DD`；未提供時取最新資料。
    pub(super) date: Option<String>,
    /// 市場：`all`（預設）、`twse` 或 `tpex`。
    #[param(value_type = MarketParamValue, inline, default = "all")]
    pub(super) market: Option<String>,
    /// 可選的正整數產業分類編號。
    #[param(minimum = 1)]
    pub(super) industry_id: Option<i32>,
    /// 最多回傳筆數，預設 20，範圍 1–50。
    #[param(minimum = 1, maximum = 50, default = 20)]
    pub(super) limit: Option<u8>,
}

/// 條件選股 endpoint 的固定白名單 query string。
///
/// 所有數值欄位先在 handler 驗證範圍，再轉成 PostgreSQL `NUMERIC` 綁定值；
/// `sort_by` 與 `sort_order` 只會映射到程式內建的十二個排序分支。
#[derive(Debug, Deserialize, IntoParams)]
pub(super) struct StockScreeningParams {
    /// 市場：`all`（預設）、`twse` 或 `tpex`。
    #[param(value_type = MarketParamValue, inline, default = "all")]
    pub(super) market: Option<String>,
    /// 可選的正整數產業分類編號。
    #[param(minimum = 1)]
    pub(super) industry_id: Option<i32>,
    /// 估值區間固定 enum。
    #[param(value_type = ValuationBandParamValue, inline)]
    pub(super) valuation_band: Option<String>,
    /// 最低營收年增率百分比，範圍 -100–10000。
    #[param(minimum = -100, maximum = 10000)]
    pub(super) min_revenue_yoy_percent: Option<f64>,
    /// 最低每股盈餘，範圍 -10000–10000。
    #[param(minimum = -10000, maximum = 10000)]
    pub(super) min_eps: Option<f64>,
    /// 最低股東權益報酬率百分比，範圍 -10000–10000。
    #[param(minimum = -10000, maximum = 10000)]
    pub(super) min_roe_percent: Option<f64>,
    /// 最低殖利率百分比，範圍 0–1000。
    #[param(minimum = 0, maximum = 1000)]
    pub(super) min_dividend_yield_percent: Option<f64>,
    /// 排序欄位固定 enum；預設 `stock_symbol`。
    #[param(value_type = StockScreenSortValue, inline, default = "stock_symbol")]
    pub(super) sort_by: Option<String>,
    /// 排序方向：`asc`（預設）或 `desc`。
    #[param(value_type = SortOrderParamValue, inline, default = "asc")]
    pub(super) sort_order: Option<String>,
    /// 最多回傳筆數，預設 20，範圍 1–50。
    #[param(minimum = 1, maximum = 50, default = 20)]
    pub(super) limit: Option<u8>,
}
