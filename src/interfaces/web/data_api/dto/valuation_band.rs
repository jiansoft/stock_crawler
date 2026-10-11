//! `/stocks/{symbol}/valuation-band` 的 request、response 與 OpenAPI schema。
//!
//! 本益比、股價淨值比在近幾年每日數值中的相對位置；計算規則見
//! [`crate::domain::quote::valuation_band`]。價格單位為新台幣元。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::domain::quote::valuation_band::{ImpliedPrices, RatioBand};

/// 估值歷史區間 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct ValuationBandParams {
    /// 回看幾年（以最新收盤日往回推），預設 5，範圍 1–10。
    #[param(minimum = 1, maximum = 10, default = 5)]
    pub(crate) years: Option<u8>,
}

/// 估值倍數回到各分位時對應的股價（假設獲利或淨值不變）。
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
pub(crate) struct ImpliedPriceBand {
    /// 倍數回到第 25 百分位時的股價。
    pub(crate) p25: f64,
    /// 倍數回到中位數時的股價。
    pub(crate) median: f64,
    /// 倍數回到第 75 百分位時的股價。
    pub(crate) p75: f64,
}

/// 單一估值倍數（本益比或股價淨值比）的歷史區間。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct RatioBandItem {
    /// 最新一日的倍數；虧損或未提供（≤ 0）時為 `null`。
    pub(crate) current: Option<f64>,
    /// 目前值在期間樣本中的百分位（0–100，樣本中小於或等於目前值的比例）；
    /// `current` 為 `null` 時也是 `null`。
    pub(crate) percentile: Option<f64>,
    /// 期間最低值。
    pub(crate) min: f64,
    /// 第 25 百分位。
    pub(crate) p25: f64,
    /// 中位數。
    pub(crate) median: f64,
    /// 第 75 百分位。
    pub(crate) p75: f64,
    /// 期間最高值。
    pub(crate) max: f64,
    /// 有效樣本天數（倍數 > 0）。
    pub(crate) samples: usize,
    /// 倍數 ≤ 0（虧損、未提供）而排除的天數。
    pub(crate) excluded_days: usize,
    /// 倍數回到各分位時對應的股價；`current` 為 `null` 時為 `null`。
    pub(crate) implied_prices: Option<ImpliedPriceBand>,
}

impl From<RatioBand> for RatioBandItem {
    fn from(band: RatioBand) -> Self {
        Self {
            current: band.current,
            percentile: band.percentile,
            min: band.min,
            p25: band.p25,
            median: band.median,
            p75: band.p75,
            max: band.max,
            samples: band.samples,
            excluded_days: band.excluded,
            implied_prices: band
                .implied_prices
                .map(|prices: ImpliedPrices| ImpliedPriceBand {
                    p25: prices.p25,
                    median: prices.median,
                    p75: prices.p75,
                }),
        }
    }
}

/// 估值歷史區間結果。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ValuationBandResponse {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 最新一筆收盤的日期；沒有日線時為 `null`。
    pub(crate) data_as_of: Option<String>,
    /// 回看年數。
    pub(crate) years: u8,
    /// 期間內最早一筆日線的日期；沒有日線時為 `null`。
    pub(crate) from: Option<String>,
    /// 最新收盤價。
    pub(crate) closing_price: Option<f64>,
    /// 本益比（交易所公布值）區間；期間內沒有任何正值時為 `null`（例如 ETF 或長期虧損）。
    pub(crate) price_earning_ratio: Option<RatioBandItem>,
    /// 股價淨值比（收盤價 ÷ 當時每股淨值）區間；期間內沒有任何正值時為 `null`。
    pub(crate) price_to_book_ratio: Option<RatioBandItem>,
}
