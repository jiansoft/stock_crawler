use axum::{
    Json, Router,
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect},
    routing::{get, post},
};

use super::dto::{ErrorResponse, INDEX_HTML};
use super::state::{BACKFILL_STATE, BackfillWebState, get_backfill_job, list_backfill_jobs};

/// 建立 backfill admin 的 Web UI 與 JSON API router。
///
/// 路由包含：
/// - `GET /manual-backfill`：操作頁面。
/// - `GET /api/manual-backfill/jobs`：列出所有 job。
/// - `GET /api/manual-backfill/jobs/{id}`：查詢單一 job。
/// - `POST /api/manual-backfill/*`：建立不同類型的回補 job。
/// - `/api/strategy-presets`：具名策略的列出、新增或覆蓋、刪除。
pub fn router() -> Router {
    Router::new()
        .route("/", get(|| async { Redirect::to("/manual-backfill") }))
        .route("/manual-backfill", get(index))
        .route("/api/manual-backfill/jobs", get(list_jobs))
        .route("/api/manual-backfill/jobs/events", get(job_events))
        .route(
            "/api/manual-backfill/jobs/{id}/events",
            get(single_job_events),
        )
        .route("/api/manual-backfill/jobs/{id}", get(get_job))
        .route(
            "/api/manual-backfill/daily-quotes",
            post(start_daily_quotes),
        )
        .route(
            "/api/manual-backfill/closing-aggregate",
            post(start_closing_aggregate),
        )
        .route(
            "/api/manual-backfill/taiwan-stock-index",
            post(start_taiwan_stock_index),
        )
        .route(
            "/api/manual-backfill/received-dividend-records",
            post(start_received_dividend_records),
        )
        .route(
            "/api/manual-backfill/historical-dividends",
            post(start_historical_dividends),
        )
        .route(
            "/api/manual-backfill/multiple-dividend-historical-dividends",
            post(start_multiple_dividend_historical_dividends),
        )
        .route(
            "/api/manual-backfill/annual-total-repair",
            post(start_annual_total_repair),
        )
        .route(
            "/api/manual-backfill/quote-history",
            post(start_quote_history),
        )
        .route(
            "/api/manual-backfill/corporate-action",
            post(save_corporate_action),
        )
        .route("/api/manual-backfill/cagr", post(start_cagr))
        .route("/api/manual-backfill/cagr-period", post(start_cagr_period))
        .with_state(BACKFILL_STATE.clone())
        .merge(super::strategy_presets::router())
}

/// 回傳 manual backfill 操作頁 HTML。
async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

/// 列出目前程序記憶體中的所有 manual backfill jobs。
async fn list_jobs(State(_state): State<BackfillWebState>) -> impl IntoResponse {
    Json(list_backfill_jobs().await)
}

/// SSE 先送最新快照，再於狀態或階段更新時推播；重連同樣取得最新狀態。
async fn job_events(State(state): State<BackfillWebState>) -> axum::response::Response {
    stream_jobs(state, None).into_response()
}

/// 個別工作完成時送 complete 並結束串流；不存在的工作立即回 404。
async fn single_job_events(
    State(state): State<BackfillWebState>,
    Path(id): Path<String>,
) -> axum::response::Response {
    if !state.jobs.read().await.contains_key(&id) {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "job not found".to_string(),
            }),
        )
            .into_response();
    }
    stream_jobs(state, Some(id)).into_response()
}

fn stream_jobs(
    state: BackfillWebState,
    id: Option<String>,
) -> axum::response::Sse<
    impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    let receiver = state.events.subscribe();
    let stream = futures::stream::unfold(
        (state, id, receiver, true, false),
        |(state, id, mut receiver, first, done)| async move {
            if done {
                return None;
            }
            if !first {
                match receiver.recv().await {
                    Ok(()) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                }
            }
            let jobs = state.jobs.read().await;
            let (event, done) = if let Some(id) = &id {
                match jobs.get(id) {
                    Some(job) => {
                        let done = job.finished_at.is_some();
                        (
                            Event::default()
                                .event(if done { "complete" } else { "progress" })
                                .json_data(job)
                                .expect("job 可序列化"),
                            done,
                        )
                    }
                    None => (Event::default().event("expired").data("{}"), true),
                }
            } else {
                let mut all: Vec<_> = jobs.values().cloned().collect();
                all.sort_by(|a, b| b.started_at.cmp(&a.started_at));
                (
                    Event::default()
                        .event("jobs")
                        .json_data(all)
                        .expect("jobs 可序列化"),
                    false,
                )
            };
            drop(jobs);
            Some((Ok(event), (state, id, receiver, false, done)))
        },
    );
    Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)))
}

/// 依 job id 查詢單一 manual backfill job。
async fn get_job(
    State(_state): State<BackfillWebState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match get_backfill_job(&id).await {
        Some(job) => Json(job).into_response(),
        None => (
            axum::http::StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("job not found: {id}"),
            }),
        )
            .into_response(),
    }
}

mod start;

use self::start::{
    save_corporate_action, start_annual_total_repair, start_cagr, start_cagr_period,
    start_closing_aggregate, start_daily_quotes, start_historical_dividends,
    start_multiple_dividend_historical_dividends, start_quote_history,
    start_received_dividend_records, start_taiwan_stock_index,
};

#[cfg(test)]
mod tests;
