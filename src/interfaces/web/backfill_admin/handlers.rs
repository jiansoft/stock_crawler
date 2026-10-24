use axum::{
    Json, Router,
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect},
    routing::{get, post},
};

use super::dto::{
    CagrPeriodRequest, CagrRequest, ClosingAggregateRequest, CorporateActionItem,
    CorporateActionRequest, CorporateActionResponse, DailyQuotesRequest, ErrorResponse, INDEX_HTML,
    QuoteHistoryRequest, SecurityCodeRequest, StartJobResponse, TaiwanStockIndexRequest,
    YearRequest,
};
use super::job_runner::{
    start_annual_total_repair_job, start_cagr_job, start_cagr_period_job,
    start_closing_aggregate_job, start_daily_quotes_job, start_historical_dividends_job,
    start_multiple_dividend_historical_dividends_job, start_quote_history_job,
    start_received_dividend_records_job, start_taiwan_stock_index_job,
};
use super::request::{
    parse_request_date, parse_request_month, parse_request_period, parse_request_security_code,
    parse_request_share_ratio, parse_request_symbol_list, start_job_error_response,
};
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

/// 建立各股每日收盤報價回補 job 的 HTTP handler。
async fn start_daily_quotes(
    State(_state): State<BackfillWebState>,
    Json(req): Json<DailyQuotesRequest>,
) -> impl IntoResponse {
    // 先驗證日期格式，避免背景 job 才因輸入錯誤失敗。
    let date = match parse_request_date(&req.date) {
        Ok(date) => date,
        Err(response) => return response,
    };

    // 輸入有效時建立背景 job，實際資料抓取與原子替換會在 job 中執行。
    // 建立可能被拒絕（相同 job 執行中 → 409、併行已滿 → 429）。
    match start_daily_quotes_job(date).await {
        Ok(job) => Json(StartJobResponse { job }).into_response(),
        Err(err) => start_job_error_response(err),
    }
}

/// 建立歷史日報價回補 job 的 HTTP handler。
async fn start_quote_history(
    State(_state): State<BackfillWebState>,
    Json(req): Json<QuoteHistoryRequest>,
) -> impl IntoResponse {
    let from = match parse_request_month(&req.from) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let to = match parse_request_month(&req.to) {
        Ok(value) => value,
        Err(response) => return response,
    };
    // 空清單代表「全部 ETF」，實際代號在 job 內解析。
    let stock_symbols = match parse_request_symbol_list(&req.stock_symbols) {
        Ok(value) => value,
        Err(response) => return response,
    };

    match start_quote_history_job(stock_symbols, from, to).await {
        Ok(job) => Json(StartJobResponse { job }).into_response(),
        Err(err) => start_job_error_response(err),
    }
}

/// 登錄公司行動（分割／減資）的 HTTP handler。
///
/// 這不是背景 job：寫入一筆對照資料是瞬間完成的，包成 job 只會讓使用者
/// 多一次輪詢才知道結果。回應直接帶回該股目前已登錄的全部事件，方便核對。
async fn save_corporate_action(
    State(_state): State<BackfillWebState>,
    Json(req): Json<CorporateActionRequest>,
) -> impl IntoResponse {
    use crate::domain::performance::repository::CorporateActionRepository;

    let stock_symbol = match parse_request_security_code(req.stock_symbol) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let effective_date = match parse_request_date(&req.effective_date) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let share_ratio = match parse_request_share_ratio(&req.share_ratio) {
        Ok(value) => value,
        Err(response) => return response,
    };

    let repository =
        crate::infra::database::repository::corporate_action::PgCorporateActionRepository::new();
    let action = crate::domain::performance::CorporateAction {
        stock_symbol: stock_symbol.clone(),
        effective_date,
        // 手動登錄介面只收比例，沒有型別欄位，只能沿用比例推斷這條退路。
        // 已知限制見 CorporateActionType::infer_from_ratio 的說明。
        action_type: crate::domain::performance::CorporateActionType::infer_from_ratio(share_ratio),
        share_ratio,
        note: req.note.trim().to_owned(),
    };

    if let Err(why) = repository.save(&action).await {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("failed to save corporate action: {why:#}"),
            }),
        )
            .into_response();
    }

    match repository.fetch_by_symbol(&stock_symbol).await {
        Ok(actions) => Json(CorporateActionResponse {
            stock_symbol,
            actions: actions
                .into_iter()
                .map(|item| CorporateActionItem {
                    effective_date: item.effective_date.to_string(),
                    share_ratio: item.share_ratio.normalize().to_string(),
                    note: item.note,
                })
                .collect(),
        })
        .into_response(),
        Err(why) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("failed to read back corporate actions: {why:#}"),
            }),
        )
            .into_response(),
    }
}

/// 建立 CAGR 重算 job 的 HTTP handler。
async fn start_cagr(
    State(_state): State<BackfillWebState>,
    Json(req): Json<CagrRequest>,
) -> impl IntoResponse {
    // 留空表示採用資料庫中最新的交易日。
    let date = if req.date.trim().is_empty() {
        None
    } else {
        match parse_request_date(&req.date) {
            Ok(value) => Some(value),
            Err(response) => return response,
        }
    };

    match start_cagr_job(date).await {
        Ok(job) => Json(StartJobResponse { job }).into_response(),
        Err(err) => start_job_error_response(err),
    }
}

/// 建立單一統計期間歷史回填 job 的 HTTP handler。
async fn start_cagr_period(
    State(_state): State<BackfillWebState>,
    Json(req): Json<CagrPeriodRequest>,
) -> impl IntoResponse {
    let period = match parse_request_period(&req.period) {
        Ok(value) => value,
        Err(response) => return response,
    };

    match start_cagr_period_job(period).await {
        Ok(job) => Json(StartJobResponse { job }).into_response(),
        Err(err) => start_job_error_response(err),
    }
}

/// 建立收盤彙總回補 job 的 HTTP handler。
async fn start_closing_aggregate(
    State(_state): State<BackfillWebState>,
    Json(req): Json<ClosingAggregateRequest>,
) -> impl IntoResponse {
    // 先驗證日期格式，避免背景 job 才因輸入錯誤失敗。
    let date = match parse_request_date(&req.date) {
        Ok(date) => date,
        Err(response) => return response,
    };

    // 輸入有效時建立背景 job，立即回傳 job 狀態給呼叫端輪詢。
    // 建立可能被拒絕（相同 job 執行中 → 409、併行已滿 → 429）。
    match start_closing_aggregate_job(date).await {
        Ok(job) => Json(StartJobResponse { job }).into_response(),
        Err(err) => start_job_error_response(err),
    }
}

/// 建立台股加權指數回補 job 的 HTTP handler。
async fn start_taiwan_stock_index(
    State(_state): State<BackfillWebState>,
    Json(req): Json<TaiwanStockIndexRequest>,
) -> impl IntoResponse {
    // 先驗證日期格式，避免背景 job 才因輸入錯誤失敗。
    let date = match parse_request_date(&req.date) {
        Ok(date) => date,
        Err(response) => return response,
    };

    // 輸入有效時建立背景 job，只會 upsert 指定日期的指數資料。
    // 建立可能被拒絕（相同 job 執行中 → 409、併行已滿 → 429）。
    match start_taiwan_stock_index_job(date).await {
        Ok(job) => Json(StartJobResponse { job }).into_response(),
        Err(err) => start_job_error_response(err),
    }
}

/// 建立持股已領股利回補 job 的 HTTP handler。
async fn start_received_dividend_records(
    State(_state): State<BackfillWebState>,
    Json(req): Json<SecurityCodeRequest>,
) -> impl IntoResponse {
    // 正規化證券代號，確保後續 crawler/database 查詢拿到乾淨輸入。
    let security_code = match parse_request_security_code(req.security_code) {
        Ok(security_code) => security_code,
        Err(response) => return response,
    };
    // 建立背景 job，HTTP request 不等待實際回補流程完成。
    // 建立可能被拒絕（相同 job 執行中 → 409、併行已滿 → 429）。
    match start_received_dividend_records_job(security_code).await {
        Ok(job) => Json(StartJobResponse { job }).into_response(),
        Err(err) => start_job_error_response(err),
    }
}

/// 建立歷年股利回補 job 的 HTTP handler。
async fn start_historical_dividends(
    State(_state): State<BackfillWebState>,
    Json(req): Json<SecurityCodeRequest>,
) -> impl IntoResponse {
    // 歷年股利回補只接受簡單英數證券代號，避免把任意字串帶入外部查詢。
    let security_code = match parse_request_security_code(req.security_code) {
        Ok(security_code) => security_code,
        Err(response) => return response,
    };
    // 建立背景 job，回補結果會更新到 job message。
    // 建立可能被拒絕（相同 job 執行中 → 409、併行已滿 → 429）。
    match start_historical_dividends_job(security_code).await {
        Ok(job) => Json(StartJobResponse { job }).into_response(),
        Err(err) => start_job_error_response(err),
    }
}

/// 建立多次配息股票歷年股利批次回補 job 的 HTTP handler。
async fn start_multiple_dividend_historical_dividends(
    State(_state): State<BackfillWebState>,
    Json(req): Json<YearRequest>,
) -> impl IntoResponse {
    // 年度資料表查詢預期使用合理西元年，先擋掉明顯輸入錯誤。
    if !(1900..=3000).contains(&req.year) {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "year must be between 1900 and 3000".to_string(),
            }),
        )
            .into_response();
    }

    // 建立背景 job，實際批次 Yahoo 回補會在 job 中執行。
    // 建立可能被拒絕（相同 job 執行中 → 409、併行已滿 → 429）。
    match start_multiple_dividend_historical_dividends_job(req.year).await {
        Ok(job) => Json(StartJobResponse { job }).into_response(),
        Err(err) => start_job_error_response(err),
    }
}

/// 建立年度合計列修復 job。
///
/// 沒有輸入參數：受影響的股票與年度由 job 自行掃描，因此不解析 request body。
async fn start_annual_total_repair(State(_state): State<BackfillWebState>) -> impl IntoResponse {
    match start_annual_total_repair_job().await {
        Ok(job) => Json(StartJobResponse { job }).into_response(),
        Err(err) => start_job_error_response(err),
    }
}

#[cfg(test)]
mod tests;
