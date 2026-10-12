//! `/stocks/{symbol}/peers` 的 request、response 與 OpenAPI schema。
//!
//! 同產業（上市櫃、未下市、不含 ETF）個股的估值、殖利率、財務體質與營收成長，
//! 以及目標股在同業間的名次；統計規則見 [`crate::domain::quote::peer`]。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// 同業比較 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct PeersParams {
    /// `peers` 最多列出幾檔（依市值由大到小），預設 20，範圍 1–100；中位數與名次一律用全部同業計算。
    #[param(minimum = 1, maximum = 100, default = 20)]
    pub(crate) limit: Option<u16>,
}

/// 單一同業個股的指標；沒有資料或已過期的欄位為 `null`。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct PeerStock {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 市場別（2 上市、4 上櫃）。
    pub(crate) market_id: i32,
    /// 最新收盤價（元）。
    pub(crate) closing_price: Option<f64>,
    /// 市值（元，收盤價 × 發行股數）。
    pub(crate) market_cap: Option<f64>,
    /// 本益比；虧損或未提供時為 `null`。
    pub(crate) price_earning_ratio: Option<f64>,
    /// 股價淨值比；未提供時為 `null`。
    pub(crate) price_to_book_ratio: Option<f64>,
    /// 殖利率（%，近 30 天內的資料）。
    pub(crate) dividend_yield_percent: Option<f64>,
    /// Piotroski F 分數（最近 2 個年度內的評分）。
    pub(crate) f_score: Option<i16>,
    /// 單季毛利率（%，最近 2 季內）。
    pub(crate) gross_margin_percent: Option<f64>,
    /// 最新月營收年增率（%，最近 3 個月內）。
    pub(crate) revenue_yoy_percent: Option<f64>,
}

/// 單一指標在同業間的位置。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct PeerMetric {
    /// 指標名稱，與 [`PeerStock`] 的欄位名稱相同。
    pub(crate) metric: String,
    /// 比較方向：`lower` 越低越好（本益比、股價淨值比）、`higher` 越高越好。
    pub(crate) better: String,
    /// 目標股的值；沒有有效值時為 `null`。
    pub(crate) target: Option<f64>,
    /// 同業中位數；沒有樣本時為 `null`。
    pub(crate) industry_median: Option<f64>,
    /// 目標股的名次（1 起算、同值同名次）；目標沒有有效值時為 `null`。
    pub(crate) rank: Option<usize>,
    /// 有效樣本數（含目標股；本益比、股價淨值比只算 > 0 的）。
    pub(crate) samples: usize,
}

/// 同業比較回應。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct PeersResponse {
    /// 查詢的股票代號。
    pub(crate) stock_symbol: String,
    /// 產業分類 id。
    pub(crate) industry_id: i32,
    /// 產業名稱；未分類時為空字串。
    pub(crate) industry_name: String,
    /// 同業報價的最新日期；沒有任何報價時為 `null`。
    pub(crate) data_as_of: Option<String>,
    /// 同業總檔數（含目標股，只算近 30 天有成交的）。
    pub(crate) peer_count: usize,
    /// 目標股的指標；近 30 天沒有報價時為 `null`。
    pub(crate) target: Option<PeerStock>,
    /// 各指標的產業中位數與目標名次。
    pub(crate) metrics: Vec<PeerMetric>,
    /// 市值最大的前 `limit` 檔同業（可能包含目標股）。
    pub(crate) peers: Vec<PeerStock>,
}
