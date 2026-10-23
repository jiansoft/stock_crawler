//! 持股穿透曝險（`/portfolio/etf-exposure`）的 request 與 response。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// 持股穿透的 query string。
#[derive(Debug, Default, Deserialize, IntoParams)]
pub(crate) struct PortfolioEtfExposureParams {
    /// 會員編號；0（預設）為全部會員合計。
    #[param(minimum = 0, default = 0)]
    pub(crate) member_id: Option<i64>,
    /// 回傳前幾檔成分股，預設 20，範圍 1–200。
    #[param(minimum = 1, maximum = 200, default = 20)]
    pub(crate) limit: Option<u16>,
}

/// 經由某檔 ETF 持有的金額。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct ExposureSourceItem {
    /// ETF 代號。
    pub(crate) etf_symbol: String,
    /// 金額（元）。
    pub(crate) value: f64,
}

/// 一檔成分股的合計曝險。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct ExposureItem {
    /// 台股代號；債券、現金等沒有代號為空字串。
    pub(crate) symbol: String,
    /// 名稱。
    pub(crate) name: String,
    /// 直接持有的市值（元）。
    pub(crate) direct_value: f64,
    /// 經由 ETF 持有的金額（元）。
    pub(crate) etf_value: f64,
    /// 合計（元）。
    pub(crate) total_value: f64,
    /// 合計占持股總市值（%）。
    pub(crate) weight: f64,
    /// 經由哪些 ETF，依金額由大到小。
    pub(crate) sources: Vec<ExposureSourceItem>,
}

/// 一檔持有 ETF 的拆解程度。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct ExposureEtfItem {
    /// ETF 代號。
    pub(crate) stock_symbol: String,
    /// 名稱。
    pub(crate) name: String,
    /// 市值（元）。
    pub(crate) market_value: f64,
    /// 前十大持股的資料日期；沒有前十大（反向、期貨型）時為 `null`。
    pub(crate) holdings_as_of: Option<String>,
    /// 前十大持股的權重合計（%），即可拆解的比例。
    pub(crate) covered_weight: f64,
}

/// 持股穿透曝險。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct PortfolioEtfExposureResponse {
    /// 有收盤價的部位市值合計（元）。
    pub(crate) total_value: f64,
    /// 其中 ETF 的市值合計（元）。
    pub(crate) etf_value: f64,
    /// ETF 前十大以外、無法拆解的金額（元）。
    pub(crate) uncovered_value: f64,
    /// 成分股總檔數（未套用 limit）。
    pub(crate) total_items: usize,
    /// 持有的 ETF，依市值由大到小。
    pub(crate) etfs: Vec<ExposureEtfItem>,
    /// 合計曝險前幾大的成分股（直接持股與經由 ETF），依合計金額由大到小。
    pub(crate) items: Vec<ExposureItem>,
    /// 沒有收盤價、不列入計算的代號。
    pub(crate) unpriced: Vec<String>,
}
