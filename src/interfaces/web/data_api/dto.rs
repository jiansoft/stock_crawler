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
