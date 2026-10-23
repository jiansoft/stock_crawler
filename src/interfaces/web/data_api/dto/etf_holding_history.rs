//! ETF 持股歷史（`/etfs/{symbol}/holdings/history`）的 request 與 response。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::EtfHoldingItem;

/// ETF 持股歷史的 query string。
#[derive(Debug, Default, Deserialize, IntoParams)]
pub(crate) struct EtfHoldingHistoryParams {
    /// 回傳最近幾期，預設 6，範圍 1–24（Yahoo 約每月一期）。
    #[param(minimum = 1, maximum = 24, default = 6)]
    pub(crate) periods: Option<u8>,
}

/// 新進或剔除的一檔持股。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct EtfHoldingWeightItem {
    /// 名稱（有台股代號時為「代號 名稱」）。
    pub(crate) label: String,
    /// 比例（%）：新進為這一期、剔除為前一期。
    pub(crate) weight: f64,
}

/// 兩期都在前十大、比例有變的一檔持股。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct EtfHoldingMoveItem {
    /// 名稱（有台股代號時為「代號 名稱」）。
    pub(crate) label: String,
    /// 前一期比例（%）。
    pub(crate) before: f64,
    /// 這一期比例（%）。
    pub(crate) after: f64,
}

/// 這一期與前一期的差異。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct EtfHoldingChanges {
    /// 前一期的資料日期。
    pub(crate) previous_as_of: String,
    /// 新進前十大，依比例由大到小。
    pub(crate) added: Vec<EtfHoldingWeightItem>,
    /// 掉出前十大，依前一期比例由大到小。
    pub(crate) removed: Vec<EtfHoldingWeightItem>,
    /// 比例有變（0.01 個百分點以上），依變化幅度由大到小。
    pub(crate) moved: Vec<EtfHoldingMoveItem>,
}

/// 一期的前十大持股。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct EtfHoldingPeriodItem {
    /// 持股資料日期。
    pub(crate) as_of: String,
    /// 前十大持股，依排名。
    pub(crate) holdings: Vec<EtfHoldingItem>,
    /// 與前一期的差異；沒有更早一期的紀錄時為 `null`。
    pub(crate) changes: Option<EtfHoldingChanges>,
}

/// ETF 持股歷史。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct EtfHoldingHistoryResponse {
    /// ETF 代號。
    pub(crate) stock_symbol: String,
    /// 名稱。
    pub(crate) name: String,
    /// 每一期的前十大持股，由新到舊；反向、期貨型通常沒有持股而為空。
    pub(crate) periods: Vec<EtfHoldingPeriodItem>,
}
