//! 建立各類回補 job（以及即時寫入的公司行動登錄）的 HTTP handler；路由在上層 `handlers`。

use axum::{Json, extract::State, response::IntoResponse};

use crate::interfaces::web::backfill_admin::dto::{
    CagrPeriodRequest, CagrRequest, ClosingAggregateRequest, CorporateActionItem,
    CorporateActionRequest, CorporateActionResponse, DailyQuotesRequest, ErrorResponse,
    QuoteHistoryRequest, SecurityCodeRequest, StartJobResponse, TaiwanStockIndexRequest,
    YearRequest,
};
use crate::interfaces::web::backfill_admin::job_runner::{
    start_annual_total_repair_job, start_cagr_job, start_cagr_period_job,
    start_closing_aggregate_job, start_daily_quotes_job, start_historical_dividends_job,
    start_multiple_dividend_historical_dividends_job, start_quote_history_job,
    start_received_dividend_records_job, start_taiwan_stock_index_job,
};
use crate::interfaces::web::backfill_admin::request::{
    parse_request_date, parse_request_month, parse_request_period, parse_request_security_code,
    parse_request_share_ratio, parse_request_symbol_list, start_job_error_response,
};
use crate::interfaces::web::backfill_admin::state::BackfillWebState;

/// 建立各股每日收盤報價回補 job 的 HTTP handler。
pub(super) async fn start_daily_quotes(
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
pub(super) async fn start_quote_history(
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
pub(super) async fn save_corporate_action(
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
pub(super) async fn start_cagr(
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
pub(super) async fn start_cagr_period(
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
pub(super) async fn start_closing_aggregate(
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
pub(super) async fn start_taiwan_stock_index(
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
pub(super) async fn start_received_dividend_records(
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
pub(super) async fn start_historical_dividends(
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
pub(super) async fn start_multiple_dividend_historical_dividends(
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
pub(super) async fn start_annual_total_repair(
    State(_state): State<BackfillWebState>,
) -> impl IntoResponse {
    match start_annual_total_repair_job().await {
        Ok(job) => Json(StartJobResponse { job }).into_response(),
        Err(err) => start_job_error_response(err),
    }
}
