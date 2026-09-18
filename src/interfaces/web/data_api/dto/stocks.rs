//! `/stocks/*` 的股票基本資料與報價 request、response 與 OpenAPI schema。
//!
//! 涵蓋股票主檔、最新日報價、歷史日線、歷史高低點與近即時快照。
//! 所有缺值欄位皆保留 `Option`，讓 serde 輸出 JSON `null` 而非猜測成零值。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// 股票基本資料。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct Stock {
    /// 系統內股票代號。
    pub(crate) stock_symbol: String,
    /// 證券代號。
    pub(crate) security_code: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 交易所市場編號。
    pub(crate) stock_exchange_market_id: i32,
    /// 產業分類編號。
    pub(crate) stock_industry_id: i32,
    /// 是否暫停交易或下市。
    pub(crate) suspend_listing: bool,
}

/// 最新單一交易日的日報價。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct DailyQuote {
    /// 交易日期，格式為 `YYYY-MM-DD`。
    pub(crate) date: String,
    /// 開盤價。
    pub(crate) opening_price: Option<f64>,
    /// 最高價。
    pub(crate) highest_price: Option<f64>,
    /// 最低價。
    pub(crate) lowest_price: Option<f64>,
    /// 收盤價。
    pub(crate) closing_price: Option<f64>,
    /// 漲跌金額。
    pub(crate) change: Option<f64>,
    /// 漲跌幅。
    pub(crate) change_range: Option<f64>,
    /// 成交股數。
    pub(crate) trading_volume: Option<f64>,
    /// 成交筆數。
    pub(crate) transaction: Option<f64>,
    /// 成交金額。
    pub(crate) trade_value: Option<f64>,
    /// 五日均線。
    pub(crate) moving_average_5: Option<f64>,
    /// 十日均線。
    pub(crate) moving_average_10: Option<f64>,
    /// 二十日均線。
    pub(crate) moving_average_20: Option<f64>,
    /// 六十日均線。
    pub(crate) moving_average_60: Option<f64>,
    /// 一百二十日均線。
    pub(crate) moving_average_120: Option<f64>,
    /// 二百四十日均線。
    pub(crate) moving_average_240: Option<f64>,
    /// 本益比。
    pub(crate) price_earning_ratio: Option<f64>,
    /// 資料紀錄時間，UTC ISO 8601。
    pub(crate) record_time: Option<String>,
    /// 最後更新時間，UTC ISO 8601。
    pub(crate) updated_time: Option<String>,
}

/// 歷史日線資料。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct HistoricalQuote {
    /// 交易日期，格式為 `YYYY-MM-DD`。
    pub(crate) date: String,
    /// 開盤價。
    pub(crate) opening_price: Option<f64>,
    /// 最高價。
    pub(crate) highest_price: Option<f64>,
    /// 最低價。
    pub(crate) lowest_price: Option<f64>,
    /// 收盤價。
    pub(crate) closing_price: Option<f64>,
    /// 漲跌金額。
    pub(crate) change: Option<f64>,
    /// 漲跌幅。
    pub(crate) change_range: Option<f64>,
    /// 成交股數。
    pub(crate) trading_volume: Option<f64>,
    /// 成交筆數。
    pub(crate) transaction: Option<f64>,
    /// 成交金額。
    pub(crate) trade_value: Option<f64>,
    /// 五日均線。
    pub(crate) moving_average_5: Option<f64>,
    /// 十日均線。
    pub(crate) moving_average_10: Option<f64>,
    /// 二十日均線。
    pub(crate) moving_average_20: Option<f64>,
    /// 六十日均線。
    pub(crate) moving_average_60: Option<f64>,
    /// 本益比。
    pub(crate) price_earning_ratio: Option<f64>,
    /// 股價淨值比。
    pub(crate) price_to_book_ratio: Option<f64>,
    /// 資料紀錄時間，UTC ISO 8601。
    pub(crate) record_time: Option<String>,
}

/// 系統收錄範圍內的歷史高低點。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct QuoteHistoryRecord {
    /// 歷史最高價。
    pub(crate) maximum_price: Option<f64>,
    /// 最高價日期。
    pub(crate) maximum_price_date_on: Option<String>,
    /// 歷史最低價。
    pub(crate) minimum_price: Option<f64>,
    /// 最低價日期。
    pub(crate) minimum_price_date_on: Option<String>,
    /// 歷史最高股價淨值比。
    pub(crate) maximum_price_to_book_ratio: Option<f64>,
    /// 最高股價淨值比日期。
    pub(crate) maximum_price_to_book_ratio_date_on: Option<String>,
    /// 歷史最低股價淨值比。
    pub(crate) minimum_price_to_book_ratio: Option<f64>,
    /// 最低股價淨值比日期。
    pub(crate) minimum_price_to_book_ratio_date_on: Option<String>,
}

/// 股票完整資料。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct StockProfile {
    /// 股票基本資料。
    pub(crate) stock: Stock,
    /// 最新日報價；沒有日報價時為 `null`。
    pub(crate) quote: Option<DailyQuote>,
    /// 近一季 EPS。
    pub(crate) last_one_eps: Option<f64>,
    /// 近四季 EPS 合計。
    pub(crate) last_four_eps: Option<f64>,
    /// 每股淨值。
    pub(crate) net_asset_value_per_share: Option<f64>,
    /// 股東權益報酬率。
    pub(crate) return_on_equity: Option<f64>,
    /// 權值。
    pub(crate) weight: Option<f64>,
    /// 發行股數。
    pub(crate) issued_share: Option<f64>,
    /// 歷史高低點；沒有紀錄時為 `null`。
    pub(crate) history: Option<QuoteHistoryRecord>,
}

/// 搜尋股票的成功回應。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct SearchResponse {
    /// 搜尋結果。
    pub(crate) stocks: Vec<Stock>,
}

/// 最新日報價的成功回應。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct LatestQuoteResponse {
    /// 股票基本資料。
    pub(crate) stock: Stock,
    /// 最新日報價；沒有資料時為 null。
    pub(crate) quote: Option<DailyQuote>,
}

/// 歷史日線的成功回應。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct PriceHistoryResponse {
    /// 符合範圍的歷史日線。
    pub(crate) quotes: Vec<HistoricalQuote>,
}

/// 第三方網站採集的近即時報價快照。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct RealtimeSnapshotResponse {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 成交價。
    pub(crate) price: Option<f64>,
    /// 漲跌。
    pub(crate) change: Option<f64>,
    /// 漲跌幅。
    pub(crate) change_range: Option<f64>,
    /// 開盤價。
    pub(crate) open: Option<f64>,
    /// 最高價。
    pub(crate) high: Option<f64>,
    /// 最低價。
    pub(crate) low: Option<f64>,
    /// 昨收價。
    pub(crate) last_close: Option<f64>,
    /// 成交量，單位為張。
    pub(crate) volume_lots: Option<f64>,
    /// 採集來源站點。
    pub(crate) source_site: String,
    /// 快照寫入快取的 UTC ISO 8601 時間。
    pub(crate) updated_at: String,
}

/// 搜尋 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct SearchParams {
    /// 搜尋字串，長度 1 至 100。
    pub(crate) query: String,
    /// 最多回傳筆數，預設 10。
    pub(crate) limit: Option<u8>,
}

/// 歷史日線 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct HistoryParams {
    /// 起始日期，格式 YYYY-MM-DD。
    pub(crate) from: Option<String>,
    /// 結束日期，格式 YYYY-MM-DD。
    pub(crate) to: Option<String>,
    /// 最多回傳筆數，預設 100。
    pub(crate) limit: Option<u16>,
}
