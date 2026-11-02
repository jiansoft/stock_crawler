//! Data API 請求的查詢預算：以 task-local 標記目前請求屬於一般或重型查詢，連線池路由見
//! [`super::get_connection`]。

use std::time::Duration;

/// Data API 請求使用的查詢預算：決定連線池與 PostgreSQL `statement_timeout`。
///
/// 由 Data API middleware 以 [`with_query_budget`] 設定成 task-local；作用範圍內
/// [`get_connection`](super::get_connection)、[`get_batch_connection`](super::get_batch_connection)
/// 與 [`get_tx`](super::get_tx) 改用對應的 Data API 連線池，
/// 讓異常 SQL 不會無限期佔住資料庫，而爬蟲與排程（不在作用範圍內）不受此上限影響。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryBudget {
    /// 一般行情／個股／持股查詢。
    General,
    /// 聚合分析、回測與穿透曝險。
    Heavy,
}

impl QueryBudget {
    /// 這類查詢在資料庫端的單一 statement 上限。
    pub const fn statement_timeout(self) -> Duration {
        match self {
            Self::General => Duration::from_secs(6),
            Self::Heavy => Duration::from_secs(25),
        }
    }
}

tokio::task_local! {
    static QUERY_BUDGET: QueryBudget;
}

/// 在 `budget` 的作用範圍內執行 `future`；其中的資料庫查詢改用 Data API 連線池。
///
/// 只對同一個 task 有效：`tokio::spawn` 出去的工作不會繼承，仍使用全域連線池。
pub async fn with_query_budget<F: std::future::Future>(
    budget: QueryBudget,
    future: F,
) -> F::Output {
    QUERY_BUDGET.scope(budget, future).await
}

/// 目前 task 的查詢預算；不在 Data API 請求內時為 `None`。
pub fn current_query_budget() -> Option<QueryBudget> {
    QUERY_BUDGET.try_with(|budget| *budget).ok()
}
