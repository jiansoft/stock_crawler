//! `/stocks/{symbol}/timeline` 的 request、response 與 OpenAPI schema。
//!
//! 把分散在各端點的個股事件依日期合併成一條時間軸：除權息與股利發放、減資／分割、月營收、季報、
//! 技術訊號（含相對強度穿越與 52 週新高新低）、外資／投信連續買賣超里程碑與董監持股變化。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// OpenAPI 文件使用的事件種類。
#[derive(ToSchema)]
#[schema(rename_all = "snake_case")]
#[allow(dead_code)] // 此 enum 僅提供 OpenAPI schema；runtime 仍以 String 回傳精確 422。
pub(crate) enum TimelineKindValue {
    /// 除息（現金股利）。
    ExDividend,
    /// 除權（股票股利）。
    ExRight,
    /// 現金股利發放。
    CashPayable,
    /// 股票股利發放。
    StockPayable,
    /// 減資、分割或反分割（股數變動生效日）。
    CorporateAction,
    /// 月營收（以法定公布期限次月 10 日為事件日）。
    MonthlyRevenue,
    /// 月營收創歷史新高（高於之前所有月份，至少要有 12 個月的歷史；事件日同月營收）。
    RevenueRecord,
    /// 季報（以法定公布期限為事件日：Q1 5/15、Q2 8/14、Q3 11/14、Q4 隔年 3/31）。
    QuarterlyReport,
    /// 三率三升或三率三降（Yahoo 單季財報的毛利率、營業利益率、稅後淨利率都高於或都低於去年同季；
    /// 事件日同季報；金融保險業不列）。
    MarginTrend,
    /// 技術訊號（均線、KD、MACD、RSI、相對強度穿越、52 週新高新低）。
    Signal,
    /// 外資或投信連續買超／賣超剛好達到 5、10、20 天。
    ChipStreak,
    /// 董監持股或質押比例單月變化達 1%（以申報期限次月 15 日為事件日）。
    Insider,
}

/// 個股事件時間軸的 query string。
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct StockTimelineParams {
    /// 迄日（`YYYY-MM-DD`）；省略為今天。
    pub(crate) date: Option<String>,
    /// 回看天數（日曆天，1–730，預設 90）。
    #[param(minimum = 1, maximum = 730, default = 90)]
    pub(crate) days: Option<u16>,
    /// 另外列出迄日之後幾天內已公告的除權息、股利發放與減資（0–180，預設 60），標記
    /// `upcoming: true`；0 為不列。
    #[param(minimum = 0, maximum = 180, default = 60)]
    pub(crate) upcoming_days: Option<u16>,
    /// 只列這些種類（逗號分隔，例如 `ex_dividend,monthly_revenue`，可用值見 `TimelineKindValue`）；省略為
    /// 全部。
    pub(crate) kinds: Option<String>,
    /// 最多幾筆（1–500，預設 200），由新到舊。
    #[param(minimum = 1, maximum = 500, default = 200)]
    pub(crate) limit: Option<u16>,
}

/// 時間軸上的一筆事件。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct TimelineEvent {
    /// 事件日（`YYYY-MM-DD`）；月營收、季報與董監持股為法定公布期限。
    pub(crate) date: String,
    /// 事件種類（見 `kinds` 參數）。
    pub(crate) kind: String,
    /// 一句話摘要，例如「除息：現金股利 4.5 元」。
    pub(crate) title: String,
    /// 補充說明；沒有為 `null`。
    pub(crate) detail: Option<String>,
    /// 所屬期間（股利所屬年度與期別、營收月份 `YYYY-MM`、財報 `YYYYQn`、董監持股月份）；其他為 `null`。
    pub(crate) period: Option<String>,
    /// 事件日晚於迄日（已公告、尚未發生）。
    pub(crate) upcoming: bool,
}

/// 各種類的事件筆數（不受 `limit` 影響）。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct TimelineKindCount {
    /// 事件種類。
    pub(crate) kind: String,
    /// 筆數。
    pub(crate) count: i64,
}

/// 個股事件時間軸。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct StockTimelineResponse {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: Option<String>,
    /// 起日（含）。
    pub(crate) from: String,
    /// 迄日（含）。
    pub(crate) to: String,
    /// 已公告事件列到這天（含）；`upcoming_days` 為 0 時等於 `to`。
    pub(crate) upcoming_to: String,
    /// 符合條件的事件總數（不受 `limit` 影響）。
    pub(crate) total: i64,
    /// 各種類的筆數，依 `kinds` 的固定順序；沒有事件的種類不列。
    pub(crate) counts: Vec<TimelineKindCount>,
    /// 事件，由新到舊；同一天依種類的固定順序。
    pub(crate) events: Vec<TimelineEvent>,
}
