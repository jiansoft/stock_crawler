//! `/market/dividend-calendar` 的 request、response 與 OpenAPI schema（股利行事曆）。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{DividendFillBucket, DividendFillSummary};

/// OpenAPI 文件使用的股利行事曆事件類型（§4.9）。
///
/// 台股領域背景：一筆股利公告有四個關鍵日期——「除息日」（買進後不再
/// 享有現金股利的基準日）、「除權日」（股票股利的基準日）、「現金股利
/// 發放日」與「股票股利發放日」。行事曆把每個日期各自視為一個事件。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema；runtime 仍以 String 回傳精確 422。
pub(crate) enum CalendarEventTypeValue {
    /// 除息日事件。
    ExDividend,
    /// 除權日事件。
    ExRights,
    /// 現金股利發放日事件。
    CashPayable,
    /// 股票股利發放日事件。
    StockPayable,
    /// 四種事件全部回傳。
    All,
}

/// 股利行事曆中的單一事件（§4.9）。
///
/// 同一筆股利公告若有多個日期落在查詢區間，會展開成多筆事件（每筆一個
/// `event_type`）。日期欄位在資料庫是字串且含 `-`、`尚未公布` 等無效標記，
/// 只有合法 `YYYY-MM-DD` 的日期才會產生事件。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct DividendCalendarEvent {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 事件類型：`ex_dividend`、`ex_rights`、`cash_payable` 或 `stock_payable`。
    pub(crate) event_type: String,
    /// 事件日期，格式 `YYYY-MM-DD`。
    pub(crate) event_date: String,
    /// 股利所屬年度（西元）；DB 欄位 `year_of_dividend`。
    pub(crate) dividend_year: i32,
    /// 期間標記：`A`（年度）、`H1`／`H2`（半年度）或 `Q1`–`Q4`（§3.5）。
    pub(crate) quarter: String,
    /// 現金股利合計（元）。
    pub(crate) cash_dividend: Option<f64>,
    /// 股票股利合計（元）。
    pub(crate) stock_dividend: Option<f64>,
    /// 股利合計（元）；DB 欄位 `sum`。
    pub(crate) total_dividend: Option<f64>,
    /// 除息／除權事件：該股近 5 年的填息統計（同 `/stocks/{symbol}/dividend-fill` 的 summary）；
    /// 發放日事件、期間內沒有除權息或查詢失敗時為 `null`。
    pub(crate) fill_history: Option<DividendFillSummary>,
    /// 除息事件且沒有自身填息紀錄時：以現金股利 ÷ 最新收盤算出的殖利率組別的全市場基準
    /// （同 `/market/dividend-fill-stats` 的一組）；其餘為 `null`。
    pub(crate) fill_peer: Option<DividendFillBucket>,
}

/// 股利行事曆的成功回應（§3.4 envelope）。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct DividendCalendarResponse {
    /// 混合事件沒有單一統計日期，固定為 `null`（各事件日期在每筆事件內）。
    pub(crate) data_as_of: Option<String>,
    /// 依 `event_date ASC`、同日 `stock_symbol ASC` 排序的事件。
    pub(crate) events: Vec<DividendCalendarEvent>,
}

/// 股利行事曆 endpoint 的 query string（§4.9）。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct DividendCalendarParams {
    /// 起始日期，格式 `YYYY-MM-DD`；未提供時預設查詢當日（台北時區）。
    pub(crate) from: Option<String>,
    /// 結束日期，格式 `YYYY-MM-DD`；未提供時預設 `from + 30` 天。
    /// `to - from` 不可超過 92 天（一季），避免全表匯出。
    pub(crate) to: Option<String>,
    /// 事件類型：`all`（預設）、`ex_dividend`、`ex_rights`、`cash_payable`
    /// 或 `stock_payable`。
    #[param(value_type = CalendarEventTypeValue, default = "all")]
    pub(crate) event_type: Option<String>,
    /// 最多回傳筆數，預設 50，範圍 1–200。
    #[param(minimum = 1, maximum = 200, default = 50)]
    pub(crate) limit: Option<u16>,
}
