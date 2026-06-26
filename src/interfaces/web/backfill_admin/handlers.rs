use axum::{
    Json, Router,
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect},
    routing::{get, post},
};

use super::dto::{
    ClosingAggregateRequest, DailyQuotesRequest, ErrorResponse, INDEX_HTML, SecurityCodeRequest,
    StartJobResponse, TaiwanStockIndexRequest, YearRequest,
};
use super::job_runner::{
    parse_request_date, parse_request_security_code, start_closing_aggregate_job,
    start_daily_quotes_job, start_historical_dividends_job,
    start_multiple_dividend_historical_dividends_job, start_received_dividend_records_job,
    start_taiwan_stock_index_job,
};
use super::state::{BACKFILL_STATE, BackfillWebState, get_backfill_job, list_backfill_jobs};

/// 建立 backfill admin 的 Web UI 與 JSON API router。
///
/// 路由包含：
/// - `GET /manual-backfill`：操作頁面。
/// - `GET /api/manual-backfill/jobs`：列出所有 job。
/// - `GET /api/manual-backfill/jobs/{id}`：查詢單一 job。
/// - `POST /api/manual-backfill/*`：建立不同類型的回補 job。
pub fn router() -> Router {
    Router::new()
        .route("/", get(|| async { Redirect::to("/manual-backfill") }))
        .route("/manual-backfill", get(index))
        .route("/api/manual-backfill/jobs", get(list_jobs))
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
        .with_state(BACKFILL_STATE.clone())
}

/// 回傳 manual backfill 操作頁 HTML。
async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

/// 列出目前程序記憶體中的所有 manual backfill jobs。
async fn list_jobs(State(_state): State<BackfillWebState>) -> impl IntoResponse {
    Json(list_backfill_jobs().await)
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

    // 輸入有效時建立背景 job，實際資料刪除與重抓會在 job 中執行。
    Json(StartJobResponse {
        job: start_daily_quotes_job(date).await,
    })
    .into_response()
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
    Json(StartJobResponse {
        job: start_closing_aggregate_job(date).await,
    })
    .into_response()
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
    Json(StartJobResponse {
        job: start_taiwan_stock_index_job(date).await,
    })
    .into_response()
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
    Json(StartJobResponse {
        job: start_received_dividend_records_job(security_code).await,
    })
    .into_response()
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
    Json(StartJobResponse {
        job: start_historical_dividends_job(security_code).await,
    })
    .into_response()
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
    Json(StartJobResponse {
        job: start_multiple_dividend_historical_dividends_job(req.year).await,
    })
    .into_response()
}
