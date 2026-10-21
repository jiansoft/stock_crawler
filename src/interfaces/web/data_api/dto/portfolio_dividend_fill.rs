//! `/portfolio/dividend-fill` 的參數與回應：持股近期每次除權息的填息狀態。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{DividendFillEvent, DividendFillSummary};

/// 持股填息狀態的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct PortfolioDividendFillParams {
    /// 只看除權息日在今天往前幾天內的事件，範圍 30–730；省略為 365。
    pub(crate) days: Option<u16>,
}

/// 持股的一次除權息與填息狀態。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct PortfolioDividendFillItem {
    /// 股票代號。
    pub(crate) stock_symbol: String,
    /// 股票名稱。
    pub(crate) name: String,
    /// 填息結果，欄位與 `/stocks/{symbol}/dividend-fill` 的 `events` 相同。
    #[serde(flatten)]
    pub(crate) fill: DividendFillEvent,
}

/// 持股填息狀態。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct PortfolioDividendFillResponse {
    /// 回看天數。
    pub(crate) days: u16,
    /// 所有事件的填息統計。
    pub(crate) summary: DividendFillSummary,
    /// 尚未填息的在前（離填息價越遠越前面），已填息的依除權息日由新到舊。
    pub(crate) items: Vec<PortfolioDividendFillItem>,
}
