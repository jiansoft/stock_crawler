//! 具名策略清單（`/strategy-presets`）的 response。

use serde::Serialize;
use utoipa::ToSchema;

/// 一個具名策略。
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct StrategyPresetItem {
    /// 策略名稱，回測端點以 `preset=名稱` 引用。
    pub(crate) name: String,
    /// 存的條件（`/stocks/screen/backtest` 的 query string）。
    pub(crate) query: String,
    /// 說明。
    pub(crate) description: String,
    /// 是否在每個交易日 22:10 發策略換股通知。
    pub(crate) alert: bool,
}

/// 具名策略清單。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct StrategyPresetsResponse {
    /// 全部策略，依名稱排序。
    pub(crate) presets: Vec<StrategyPresetItem>,
}
