//! `/market/signals/performance` 與 `/stocks/{symbol}/signals` handler：技術訊號的歷史表現。
//!
//! 訊號來自 `daily_technical_signal`，之後的表現來自 `daily_forward_return`（收盤後回填的
//! 5／20／60／120／240 個交易日還原報酬，見 [`crate::app::calculation::forward_return`]）。
//!
//! 子模組：[`sql`]（SQL）、[`rows`]（查詢列）、[`benchmark`]（大盤基準）、[`validation`]
//! （共用的參數驗證）、[`performance`]（訊號績效）、[`stock_signals`](mod@stock_signals)（個股訊號歷史）。

/// 統計的天期（交易日數），與 `daily_forward_return` 的欄位對應。
const HORIZONS: [u32; 5] = [5, 20, 60, 120, 240];

/// 預設統計期間：迄日往前 364 天（約一年）。
const DEFAULT_PERIOD_DAYS: i64 = 364;

/// 統計期間上限（日曆天，含起訖）：技術訊號、未來報酬、相對強度與籌碼都從 2019 年起。
const MAX_PERIOD_DAYS: i64 = 3653;

/// 不指定訊號來源時的期間上限：全部來源一次統計多年要數十秒，超過時必須指定 `indicator`。
const MAX_ALL_SIGNALS_PERIOD_DAYS: i64 = 1096;

mod benchmark;
mod fetch;
mod performance;
mod rows;
mod sql;
mod stock_signals;
mod validation;

pub(super) use self::benchmark::{benchmark_output, fetch_benchmark};
pub(crate) use self::performance::{__path_signal_performance, signal_performance};
pub(super) use self::performance::{horizon_stats, performance_years, total_row};
pub(super) use self::rows::PerformanceRow;
pub(super) use self::sql::{chip_streak_ctes, performance_aggregate_sql};
pub(crate) use self::stock_signals::{__path_stock_signals, stock_signals};
pub(super) use self::validation::{
    MAX_STREAK_DAYS, parse_group_by, resolve_period, validate_rs_range, validate_streaks,
};

#[cfg(test)]
mod tests;
