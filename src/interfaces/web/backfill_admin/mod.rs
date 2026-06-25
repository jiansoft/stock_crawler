//! Backfill admin web UI and API.
//!
//! 這個模組提供一個輕量的 Web UI 與 JSON API，讓維運人員可以手動觸發
//! 各股每日收盤報價、收盤彙總、台股加權指數、持股已領股利重算、歷年股利補抓等資料修補工作。
//! 所有工作都會先登記成 job，再由背景 task 執行，呼叫端可用 job API 查詢執行狀態。

mod dto;
mod handlers;
mod job_runner;
mod state;

pub use handlers::router;
pub(crate) use job_runner::{
    normalize_security_code, start_closing_aggregate_job, start_daily_quotes_job,
    start_historical_dividends_job, start_multiple_dividend_historical_dividends_job,
    start_received_dividend_records_job, start_taiwan_stock_index_job,
};
pub(crate) use state::{BackfillJob, get_backfill_job, list_backfill_jobs};
