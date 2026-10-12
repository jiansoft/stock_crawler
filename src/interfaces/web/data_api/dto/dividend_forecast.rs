//! `/portfolio/dividend-forecast` 的 request、response 與 OpenAPI schema。
//!
//! 以目前持股預估未來幾個月的現金股利入帳；規則見
//! [`crate::domain::portfolio::dividend_forecast`]。金額單位為新台幣元。

use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::domain::portfolio::dividend_forecast::{Basis, Forecast, ForecastItem, MonthTotal};

/// 股利現金流預估 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct DividendForecastParams {
    /// 會員編號；0（預設）為全部會員合計。
    #[param(minimum = 0, default = 0)]
    pub(crate) member_id: Option<i64>,
    /// 預估未來幾個月，預設 12，範圍 1–24。
    #[param(minimum = 1, maximum = 24, default = 12)]
    pub(crate) months: Option<u8>,
}

/// OpenAPI 文件使用的預估依據。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema。
pub(crate) enum ForecastBasisValue {
    /// 除息日與金額都已公告。
    Announced,
    /// 金額已公告、日期依去年同期推估。
    AmountAnnounced,
    /// 依去年同期推估。
    Estimated,
}

/// 單筆預估配息。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct DividendForecastItem {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 持有股數。
    pub(crate) shares: i64,
    /// 除息日（推估者為去年同期加一年）。
    pub(crate) ex_date: String,
    /// 發放日（已公告者為公告日，否則為推估）。
    pub(crate) pay_date: String,
    /// 每股現金股利（元）。
    pub(crate) cash_per_share: f64,
    /// 預估入帳金額。
    pub(crate) amount: f64,
    /// 預估依據。
    #[schema(value_type = ForecastBasisValue)]
    pub(crate) basis: String,
}

impl From<ForecastItem> for DividendForecastItem {
    fn from(item: ForecastItem) -> Self {
        Self {
            stock_symbol: item.stock_symbol,
            name: item.name,
            shares: item.shares,
            ex_date: item.ex_date.to_string(),
            pay_date: item.pay_date.to_string(),
            cash_per_share: item.cash_per_share.round_dp(4).to_f64().unwrap_or_default(),
            amount: item.amount.round_dp(2).to_f64().unwrap_or_default(),
            basis: match item.basis {
                Basis::Announced => "announced",
                Basis::AmountAnnounced => "amount_announced",
                Basis::Estimated => "estimated",
            }
            .to_owned(),
        }
    }
}

/// 單月預估合計。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct DividendForecastMonth {
    /// 月份，格式 `YYYY-MM`。
    pub(crate) month: String,
    /// 已公告（含金額已公告）的金額。
    pub(crate) announced: f64,
    /// 依去年同期推估的金額。
    pub(crate) estimated: f64,
}

impl From<MonthTotal> for DividendForecastMonth {
    fn from(month: MonthTotal) -> Self {
        Self {
            month: month.month,
            announced: month.announced,
            estimated: month.estimated,
        }
    }
}

/// 股利現金流預估結果。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct DividendForecastResponse {
    /// 會員編號；0 為全部會員合計。
    pub(crate) member_id: i64,
    /// 預估起日（不含，即查詢當天）。
    pub(crate) from: String,
    /// 預估迄日（含）。
    pub(crate) to: String,
    /// 預估期間合計（只含現金股利，未扣二代健保補充保費與稅）。
    pub(crate) total: f64,
    /// 其中已公告（含金額已公告）的金額。
    pub(crate) announced_total: f64,
    /// 其中依去年同期推估的金額。
    pub(crate) estimated_total: f64,
    /// 預估股利 ÷ 目前市值（%）；沒有市值時為 `null`。
    pub(crate) forward_yield_percent: Option<f64>,
    /// 各月合計（含 0 的月份）。
    pub(crate) months: Vec<DividendForecastMonth>,
    /// 各筆預估，依發放日排序。
    pub(crate) items: Vec<DividendForecastItem>,
}

impl DividendForecastResponse {
    /// 由領域預估組出回應。
    pub(crate) fn new(member_id: i64, forecast: Forecast) -> Self {
        Self {
            member_id,
            from: forecast.from.to_string(),
            to: forecast.to.to_string(),
            total: forecast.total,
            announced_total: forecast.announced_total,
            estimated_total: forecast.estimated_total,
            forward_yield_percent: forecast.forward_yield_percent,
            months: forecast.months.into_iter().map(Into::into).collect(),
            items: forecast.items.into_iter().map(Into::into).collect(),
        }
    }
}
