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
    parse_request_date, parse_request_month, parse_request_period, parse_request_security_code,
    parse_request_share_ratio, parse_request_symbol_list, start_cagr_job, start_cagr_period_job,
    start_closing_aggregate_job, start_daily_quotes_job, start_historical_dividends_job,
    start_job_error_response, start_multiple_dividend_historical_dividends_job,
    start_quote_history_job, start_received_dividend_records_job, start_taiwan_stock_index_job,
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

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::super::dto::INDEX_HTML;
    use super::router;

    /// 送出 JSON body 並取回狀態碼。
    async fn post(path: &str, body: &str) -> StatusCode {
        router()
            .oneshot(
                Request::post(path)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_owned()))
                    .expect("request should build"),
            )
            .await
            .expect("router should serve request")
            .status()
    }

    #[tokio::test]
    async fn invalid_input_is_rejected_before_any_job_is_created() {
        // 月份格式錯誤（缺少月份、用了日期格式）。
        assert_eq!(
            post(
                "/api/manual-backfill/quote-history",
                r#"{"stock_symbols":"0050","from":"2015","to":"2021-12"}"#
            )
            .await,
            StatusCode::BAD_REQUEST
        );
        // 代號含非英數字元。
        assert_eq!(
            post(
                "/api/manual-backfill/quote-history",
                r#"{"stock_symbols":"0050; DROP","from":"2015-01","to":"2021-12"}"#
            )
            .await,
            StatusCode::BAD_REQUEST
        );
        // 未知的期間代碼。
        assert_eq!(
            post("/api/manual-backfill/cagr-period", r#"{"period":"Y8"}"#).await,
            StatusCode::BAD_REQUEST
        );
        // 公司行動：比例為零、負數或非數字都必須擋下。
        for ratio in ["0", "-4", "1:4"] {
            assert_eq!(
                post(
                    "/api/manual-backfill/corporate-action",
                    &format!(
                        r#"{{"stock_symbol":"0050","effective_date":"2025-06-18","share_ratio":"{ratio}"}}"#
                    )
                )
                .await,
                StatusCode::BAD_REQUEST,
                "share_ratio={ratio} 應被拒絕"
            );
        }

        // CAGR 基準日格式錯誤（留空才是合法的「採用最新交易日」）。
        // 注意 parse_request_date 沿用 chrono 的寬鬆解析，"2026-8-7" 是合法的，
        // 因此這裡用真正無法解析的值。
        assert_eq!(
            post("/api/manual-backfill/cagr", r#"{"date":"not-a-date"}"#).await,
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn every_form_endpoint_in_the_page_has_a_route() {
        // 表單的 data-endpoint 與 router 若不同步，UI 會安靜地送到 404。
        for endpoint in [
            "/api/manual-backfill/daily-quotes",
            "/api/manual-backfill/closing-aggregate",
            "/api/manual-backfill/taiwan-stock-index",
            "/api/manual-backfill/received-dividend-records",
            "/api/manual-backfill/historical-dividends",
            "/api/manual-backfill/multiple-dividend-historical-dividends",
            "/api/manual-backfill/quote-history",
            "/api/manual-backfill/cagr",
            "/api/manual-backfill/cagr-period",
            "/api/manual-backfill/corporate-action",
        ] {
            assert!(
                INDEX_HTML.contains(&format!("data-endpoint=\"{endpoint}\"")),
                "頁面缺少 {endpoint} 的表單"
            );
        }
    }
}
