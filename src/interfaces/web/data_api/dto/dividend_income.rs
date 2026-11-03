//! `/portfolio/dividend-income` 的 request、response 與 OpenAPI schema。
//!
//! 持股某一年 1～12 月的股利入帳（依發放日）；金額單位為新台幣元。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::UpcomingPayoutItem;

/// 持股年度股利入帳 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct DividendIncomeParams {
    /// 年度，預設今年；範圍 2000 到明年。
    #[param(minimum = 2000)]
    pub(crate) year: Option<i32>,
    /// 會員編號；0（預設）為全部會員合計。
    #[param(minimum = 0, default = 0)]
    pub(crate) member_id: Option<i64>,
    /// `true` 時另回 `items`：今年每一筆入帳（同 `/portfolio/upcoming-payouts` 的項目）。
    #[param(default = false)]
    pub(crate) include_items: Option<bool>,
}

/// 一個月的股利入帳。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct DividendIncomeMonth {
    /// 月份（1～12）。
    pub(crate) month: u32,
    /// 現金入帳合計（未扣二代健保補充保費與稅）。
    pub(crate) cash_amount: f64,
    /// 其中發放日不晚於 `as_of`（已入帳）的現金。
    pub(crate) received_cash_amount: f64,
    /// 其中尚未除權息、以目前持股推算的現金；其餘是已除權息、金額取已領股利明細。
    pub(crate) estimated_cash_amount: f64,
    /// 配股股數（不足一股捨去）。
    pub(crate) stock_shares: i64,
    /// 入帳筆數（同一天發放的現金與股票股利算一筆）。
    pub(crate) payouts: usize,
    /// 去年同月的現金入帳。
    pub(crate) last_year_cash_amount: f64,
}

/// 持股年度股利入帳結果。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct DividendIncomeResponse {
    /// 會員編號；0 為全部會員合計。
    pub(crate) member_id: i64,
    /// 年度。
    pub(crate) year: i32,
    /// 「已入帳」的基準日（查詢當天），格式 `YYYY-MM-DD`。
    pub(crate) as_of: String,
    /// 全年現金入帳合計（含尚未發放、已公告發放日的）。
    pub(crate) total_cash_amount: f64,
    /// 其中已入帳的現金。
    pub(crate) received_cash_amount: f64,
    /// 其中以目前持股推算的現金。
    pub(crate) estimated_cash_amount: f64,
    /// 全年配股股數。
    pub(crate) total_stock_shares: i64,
    /// 去年全年的現金入帳。
    pub(crate) last_year_cash_amount: f64,
    /// 去年同期（1 月 1 日到去年的 `as_of` 同日，年度已過完時為全年）的現金入帳，與 `received_cash_amount` 對照。
    pub(crate) last_year_same_period_cash_amount: f64,
    /// 1～12 月，沒有入帳的月份為 0；未來月份只含已公告發放日的股利。
    pub(crate) months: Vec<DividendIncomeMonth>,
    /// 今年每一筆入帳，依發放日、代號排序；只有查詢帶 `include_items=true` 時才有此欄位。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) items: Option<Vec<UpcomingPayoutItem>>,
}
