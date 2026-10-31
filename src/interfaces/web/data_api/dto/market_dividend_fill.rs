//! `/market/dividend-fill-stats` 的參數與回應：全市場依現金殖利率分組的填息基準。
//!
//! 規則見 [`crate::domain::dividend::fill_stats`]；填息目標價依配股調整，與個股填息相同。

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// 全市場填息基準的 query string。
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct MarketDividendFillStatsParams {
    /// 統計今天往前幾年內的除權息，範圍 1–8；省略為 5。
    #[param(minimum = 1, maximum = 8, default = 5)]
    pub(crate) years: Option<u8>,
}

/// 一個現金殖利率分組的填息統計。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct DividendFillBucket {
    /// 分組代碼：`below_3`、`from_3_to_5`、`from_5_to_7` 或 `at_least_7`
    /// （現金股利 ÷ 除權息前一日收盤，下限含、上限不含）。
    pub(crate) bucket: String,
    /// 區間文字，例如 `3～5%`。
    pub(crate) label: String,
    /// 期間內有現金股利的除權息次數（含觀察期未滿者）。
    pub(crate) events: usize,
    /// 除權息後已滿 `mature_trading_days` 個交易日、列入填息率與中位數的次數。
    pub(crate) mature_events: usize,
    /// 20 個交易日內填息的比例（%）；沒有可統計的事件為 `null`。
    pub(crate) fill_rate_20d: Option<f64>,
    /// 60 個交易日內填息的比例（%）。
    pub(crate) fill_rate_60d: Option<f64>,
    /// 120 個交易日內填息的比例（%）。
    pub(crate) fill_rate_120d: Option<f64>,
    /// 觀察期內已填息者的填息交易日數中位數；沒有已填息者為 `null`。
    pub(crate) median_trading_days_to_fill: Option<f64>,
}

/// 全市場填息基準。
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct MarketDividendFillStatsResponse {
    /// 統計的年數。
    pub(crate) years: u8,
    /// 除權息日起日（不含），格式 `YYYY-MM-DD`。
    pub(crate) from: String,
    /// 除權息日迄日（含），格式 `YYYY-MM-DD`。
    pub(crate) to: String,
    /// 列入統計需要的觀察期（交易日）。
    pub(crate) mature_trading_days: u32,
    /// 四個殖利率分組，由低到高；沒有樣本的組次數為 0。
    pub(crate) buckets: Vec<DividendFillBucket>,
}
