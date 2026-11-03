//! `/portfolio/dividend-forecast/accuracy` 的 request、response 與 OpenAPI schema。
//!
//! 比對規則見 [`crate::domain::portfolio::forecast_accuracy`]。金額單位為新台幣元。

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::domain::portfolio::forecast_accuracy::{AccuracyItem, BasisAccuracy, ForecastAccuracy};

/// 股利預估誤差 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct DividendForecastAccuracyParams {
    /// 快照日（`YYYY-MM-DD`）；未指定時取最近一次快照。
    pub(crate) snapshot_date: Option<String>,
    /// 會員編號；0（預設）為全部會員合計。
    #[param(minimum = 0, default = 0)]
    pub(crate) member_id: Option<i64>,
}

/// OpenAPI 文件使用的比對結果。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema。
pub(crate) enum ForecastAccuracyStatusValue {
    /// 有預估也有入帳。
    Matched,
    /// 發放日還沒到。
    Pending,
    /// 有預估，但發放月份已過仍沒有入帳。
    Missed,
    /// 有入帳，但快照沒有預估（例如快照後才買進）。
    Unforecast,
}

/// 金額轉成兩位小數的 `f64`。
fn money(value: Decimal) -> f64 {
    value.round_dp(2).to_f64().unwrap_or_default()
}

/// 單一（代號, 發放月份）的預估與實際。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct DividendForecastAccuracyItem {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 發放月份，格式 `YYYY-MM`。
    pub(crate) month: String,
    /// 預估依據；同月多種依據以 `/` 串接，沒有預估時為空字串。
    pub(crate) basis: String,
    /// 預估金額。
    pub(crate) forecast: f64,
    /// 實際入帳金額；`pending` 時為 `null`。
    pub(crate) actual: Option<f64>,
    /// 實際 − 預估；`pending` 時為 `null`。
    pub(crate) difference: Option<f64>,
    /// |實際 − 預估| ÷ 實際（%）；只有 `matched` 且實際大於 0 時有值。
    pub(crate) error_percent: Option<f64>,
    /// 比對結果。
    #[schema(value_type = ForecastAccuracyStatusValue)]
    pub(crate) status: String,
}

impl From<AccuracyItem> for DividendForecastAccuracyItem {
    fn from(item: AccuracyItem) -> Self {
        Self {
            stock_symbol: item.stock_symbol,
            name: item.name,
            month: item.month.format("%Y-%m").to_string(),
            basis: item.basis,
            forecast: money(item.forecast),
            actual: item.actual.map(money),
            difference: item.difference.map(money),
            error_percent: item.error_percent.map(money),
            status: item.status.as_str().to_owned(),
        }
    }
}

/// 單一預估依據的誤差彙總。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct DividendForecastBasisAccuracy {
    /// 預估依據。
    pub(crate) basis: String,
    /// 配對成功（`matched`）的筆數。
    pub(crate) count: usize,
    /// 誤差百分比中位數。
    pub(crate) median_error_percent: Option<f64>,
}

impl From<BasisAccuracy> for DividendForecastBasisAccuracy {
    fn from(row: BasisAccuracy) -> Self {
        Self {
            basis: row.basis,
            count: row.count,
            median_error_percent: row.median_error_percent.map(money),
        }
    }
}

/// 股利預估誤差結果。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct DividendForecastAccuracyResponse {
    /// 會員編號；0 為全部會員合計。
    pub(crate) member_id: i64,
    /// 比對的快照日；還沒有任何快照時為 `null`。
    pub(crate) snapshot_date: Option<String>,
    /// 比對截止日（查詢當天）。
    pub(crate) as_of: String,
    /// 已結算（發放日已到）的預估合計。
    pub(crate) settled_forecast: f64,
    /// 已結算的實際入帳合計。
    pub(crate) settled_actual: f64,
    /// 發放日還沒到的筆數。
    pub(crate) pending_count: usize,
    /// 依預估依據彙總，依依據名稱排序。
    pub(crate) by_basis: Vec<DividendForecastBasisAccuracy>,
    /// 依發放月份、代號排序的比對結果。
    pub(crate) items: Vec<DividendForecastAccuracyItem>,
}

impl DividendForecastAccuracyResponse {
    /// 由領域比對結果組出回應；`snapshot_date` 為 `None` 時代表沒有快照。
    pub(crate) fn new(
        member_id: i64,
        snapshot_date: Option<String>,
        as_of: String,
        accuracy: ForecastAccuracy,
    ) -> Self {
        let pending_count = accuracy
            .items
            .iter()
            .filter(|item| item.actual.is_none())
            .count();
        Self {
            member_id,
            snapshot_date,
            as_of,
            settled_forecast: money(accuracy.settled_forecast),
            settled_actual: money(accuracy.settled_actual),
            pending_count,
            by_basis: accuracy.by_basis.into_iter().map(Into::into).collect(),
            items: accuracy.items.into_iter().map(Into::into).collect(),
        }
    }
}
