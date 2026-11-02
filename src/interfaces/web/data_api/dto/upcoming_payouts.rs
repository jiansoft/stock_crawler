//! `/portfolio/upcoming-payouts` 的 request、response 與 OpenAPI schema。
//!
//! 持股未來幾天的股利入帳（發放日）；金額單位為新台幣元。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// 持股近期股利入帳 endpoint 的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct UpcomingPayoutsParams {
    /// 會員編號；0（預設）為全部會員合計。
    #[param(minimum = 0, default = 0)]
    pub(crate) member_id: Option<i64>,
    /// 從今天起往後幾天（含今天），預設 30，範圍 1–90。
    #[param(minimum = 1, maximum = 90, default = 30)]
    pub(crate) days: Option<u16>,
}

/// 一檔持股在某個發放日的股利入帳（同一天發放的現金與股票股利合為一筆）。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct UpcomingPayoutItem {
    /// 發放日，格式 `YYYY-MM-DD`。
    pub(crate) payable_date: String,
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 股利期別（`Q1`～`Q4`、`H1`／`H2`、月配 `M01`…；年配為空字串）。
    pub(crate) quarter: String,
    /// 這筆入帳對應的除權息日（現金取除息日、只發股票時取除權日）；未公告為 `null`。
    pub(crate) ex_date: Option<String>,
    /// 參與分配的股數。
    pub(crate) shares: i64,
    /// 這天入帳的每股現金股利（元）；這天沒有現金入帳為 0。
    pub(crate) cash_dividend: f64,
    /// 這天入帳的每股股票股利（元）；這天沒有配股為 0。
    pub(crate) stock_dividend: f64,
    /// 現金入帳（未扣二代健保補充保費與稅）。
    pub(crate) cash_amount: f64,
    /// 配股股數（不足一股捨去）。
    pub(crate) stock_shares: i64,
    /// `false`：已除權息，金額取除權息當天寫入的已領股利明細；`true`：尚未除權息，以目前持股推算。
    pub(crate) estimated: bool,
}

/// 持股近期股利入帳結果。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct UpcomingPayoutsResponse {
    /// 會員編號；0 為全部會員合計。
    pub(crate) member_id: i64,
    /// 起日（含，即查詢當天）。
    pub(crate) from: String,
    /// 迄日（含）。
    pub(crate) to: String,
    /// 期間內現金入帳合計（未扣二代健保補充保費與稅）。
    pub(crate) total_cash_amount: f64,
    /// 依發放日、代號排序。
    pub(crate) items: Vec<UpcomingPayoutItem>,
}
