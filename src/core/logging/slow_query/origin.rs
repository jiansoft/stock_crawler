//! 慢查詢的來源：排程工作或 Data API 請求。
//!
//! 排程（[`in_job`]）與 Data API（[`in_api`]）在各自的 task 內設定來源標記；sqlx 的慢查詢事件在執行查詢的
//! 同一個 task 裡發出，[`super::observe`] 讀得到標記，寫進日誌行的 `source` 欄位並分開統計。
//! 只對同一個 task 有效：`tokio::spawn` 出去的工作不會繼承，歸為「其他」；舊日誌沒有這個欄位，也歸為「其他」。

use std::future::Future;

tokio::task_local! {
    /// 目前 task 的來源標記：`job:<排程名稱>` 或 `api:<請求路徑>`。
    static SOURCE: String;
}

/// 排程工作的標記前綴。
const JOB_PREFIX: &str = "job:";
/// Data API 請求的標記前綴。
const API_PREFIX: &str = "api:";

/// 慢查詢的來源分類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Origin {
    /// 排程工作。
    Job,
    /// Data API 請求（含 MCP 觸發的手動回測）。
    Api,
    /// 沒有標記：`tokio::spawn` 出去的工作、啟動流程，或沒有 `source` 欄位的舊日誌。
    Other,
}

impl Origin {
    /// 彙總文字用的名稱。
    pub fn label(self) -> &'static str {
        match self {
            Self::Job => "排程",
            Self::Api => "API",
            Self::Other => "其他",
        }
    }

    /// 依日誌的 `source` 欄位值分類；沒有欄位或前綴不認得時為 [`Origin::Other`]。
    pub fn from_source(source: Option<&str>) -> Self {
        match source {
            Some(value) if value.starts_with(JOB_PREFIX) => Self::Job,
            Some(value) if value.starts_with(API_PREFIX) => Self::Api,
            _ => Self::Other,
        }
    }
}

/// 在排程工作 `name` 的來源標記下執行 `future`。
pub async fn in_job<F: Future>(name: &str, future: F) -> F::Output {
    SOURCE.scope(format!("{JOB_PREFIX}{name}"), future).await
}

/// 在 Data API 請求 `path` 的來源標記下執行 `future`。
pub async fn in_api<F: Future>(path: &str, future: F) -> F::Output {
    SOURCE.scope(format!("{API_PREFIX}{path}"), future).await
}

/// 目前 task 的來源標記；不在 [`in_job`]／[`in_api`] 範圍內時為 `None`。
pub(super) fn current() -> Option<String> {
    SOURCE.try_with(Clone::clone).ok()
}
