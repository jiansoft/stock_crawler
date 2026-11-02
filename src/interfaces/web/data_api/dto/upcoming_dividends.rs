//! `/portfolio/upcoming-dividends` 的 request、response 與 OpenAPI schema。
//!
//! 目前持股未來幾天已公告的除權息；規則見 [`crate::app::calculation::upcoming_dividends`]。
//! 金額單位為新台幣元。

use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{DividendFillBucket, DividendFillSummary};
use crate::app::calculation::upcoming_dividends::UpcomingDividend;

/// 持股近期除權息 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct UpcomingDividendsParams {
    /// 會員編號；0（預設）為全部會員合計。
    #[param(minimum = 0, default = 0)]
    pub(crate) member_id: Option<i64>,
    /// 從今天起往後幾天（含今天），預設 14，範圍 1–60。
    #[param(minimum = 1, maximum = 60, default = 14)]
    pub(crate) days: Option<u16>,
}

/// 一檔持股在某天的除權息。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct UpcomingDividendItem {
    /// 除權息日，格式 `YYYY-MM-DD`（同一檔除息與除權不同天時分成兩筆）。
    pub(crate) ex_date: String,
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 持有股數。
    pub(crate) shares: i64,
    /// 這天除息的每股現金股利（元）；這天不除息為 0。
    pub(crate) cash_dividend: f64,
    /// 這天除權的每股股票股利（元）；這天不除權為 0。
    pub(crate) stock_dividend: f64,
    /// 預估現金入帳（股數 × 每股現金股利，未扣二代健保補充保費與稅）。
    pub(crate) cash_amount: f64,
    /// 預估配股股數（股票股利以面額 10 元換算，不足一股捨去）。
    pub(crate) stock_shares: i64,
    /// 該股近 5 年的填息統計（同 `/stocks/{symbol}/dividend-fill` 的 summary）；沒有紀錄時為 `null`。
    pub(crate) fill_history: Option<DividendFillSummary>,
    /// 沒有自身紀錄時，以每股現金股利 ÷ 最新收盤算出殖利率組別的全市場填息基準；其餘為 `null`。
    pub(crate) fill_peer: Option<DividendFillBucket>,
}

impl From<&UpcomingDividend> for UpcomingDividendItem {
    fn from(item: &UpcomingDividend) -> Self {
        Self {
            ex_date: item.ex_date.to_string(),
            stock_symbol: item.stock_symbol.clone(),
            name: item.name.clone(),
            shares: item.shares,
            cash_dividend: item.cash_dividend.round_dp(4).to_f64().unwrap_or_default(),
            stock_dividend: item.stock_dividend.round_dp(4).to_f64().unwrap_or_default(),
            cash_amount: item.cash_amount().round_dp(2).to_f64().unwrap_or_default(),
            stock_shares: item.stock_shares().to_i64().unwrap_or_default(),
            fill_history: None,
            fill_peer: None,
        }
    }
}

/// 持股近期除權息結果。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct UpcomingDividendsResponse {
    /// 會員編號；0 為全部會員合計。
    pub(crate) member_id: i64,
    /// 起日（含，即查詢當天）。
    pub(crate) from: String,
    /// 迄日（含）。
    pub(crate) to: String,
    /// 期間內預估現金入帳合計（未扣二代健保補充保費與稅）。
    pub(crate) total_cash_amount: f64,
    /// 依除權息日、代號排序。
    pub(crate) items: Vec<UpcomingDividendItem>,
}
