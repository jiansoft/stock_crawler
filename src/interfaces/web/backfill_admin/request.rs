//! Manual backfill HTTP API 的請求解析與錯誤回應：把 request 欄位轉成 job 參數，
//! 格式錯誤一律回 400，job 啟動衝突回 409／429。

use anyhow::{Result, anyhow};
use axum::Json;
use axum::response::IntoResponse;
use chrono::NaiveDate;

use crate::domain::performance::CagrPeriod;

use super::dto::ErrorResponse;
use super::job_runner::StartJobError;

/// 將 [`StartJobError`] 轉成一致的 HTTP 錯誤回應。
///
/// - 重複 job → `409 Conflict`：同一份工作已在執行，訊息附上既有 job id。
/// - 併行已滿 → `429 Too Many Requests`：請呼叫端稍後重試。
pub(super) fn start_job_error_response(err: StartJobError) -> axum::response::Response {
    let status = match err {
        StartJobError::DuplicateActiveJob { .. } => axum::http::StatusCode::CONFLICT,
        StartJobError::TooManyActiveJobs { .. } => axum::http::StatusCode::TOO_MANY_REQUESTS,
    };
    (
        status,
        Json(ErrorResponse {
            error: err.to_string(),
        }),
    )
        .into_response()
}

/// 解析 HTTP request 的日期欄位，格式錯誤時回傳一致的 400 response。
#[allow(clippy::result_large_err)]
pub(super) fn parse_request_date(date: &str) -> Result<NaiveDate, axum::response::Response> {
    NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d").map_err(|why| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("date must use YYYY-MM-DD: {why}"),
            }),
        )
            .into_response()
    })
}

/// 解析 HTTP request 的月份欄位（`YYYY-MM`），回傳該月第一天。
#[allow(clippy::result_large_err)]
pub(super) fn parse_request_month(month: &str) -> Result<NaiveDate, axum::response::Response> {
    // `<input type="month">` 送出的是 `YYYY-MM`，補上第一天才能解析成日期。
    NaiveDate::parse_from_str(&format!("{}-01", month.trim()), "%Y-%m-%d").map_err(|why| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("month must use YYYY-MM: {why}"),
            }),
        )
            .into_response()
    })
}

/// 解析 HTTP request 的期間代碼欄位。
#[allow(clippy::result_large_err)]
pub(super) fn parse_request_period(period: &str) -> Result<CagrPeriod, axum::response::Response> {
    CagrPeriod::from_code(period.trim()).ok_or_else(|| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("unknown period: {period}"),
            }),
        )
            .into_response()
    })
}

/// 解析公司行動的股數變動比例。
///
/// 必須大於零：`0` 會讓持股歸零、負數毫無意義，兩者都只會產生錯得離譜的
/// 報酬率。上界取 1000，擋掉把「1:4」整串貼進來之類的輸入錯誤。
#[allow(clippy::result_large_err)]
pub(super) fn parse_request_share_ratio(
    raw: &str,
) -> Result<rust_decimal::Decimal, axum::response::Response> {
    let reject = |message: String| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error: message }),
        )
            .into_response()
    };

    let ratio = rust_decimal::Decimal::from_str_exact(raw.trim())
        .map_err(|why| reject(format!("share_ratio must be a decimal number: {why}")))?;
    if ratio <= rust_decimal::Decimal::ZERO {
        return Err(reject("share_ratio must be greater than zero".to_string()));
    }
    if ratio > rust_decimal::Decimal::from(1000) {
        return Err(reject("share_ratio looks implausible (> 1000)".to_string()));
    }
    Ok(ratio)
}

/// 解析以逗號、空白或換行分隔的代號清單；空字串回傳空 `Vec`。
///
/// 空清單在呼叫端有明確語意（全部 ETF），因此不視為錯誤；
/// 但只要有填就逐一驗證，避免把打錯的代號送去打外部 API。
#[allow(clippy::result_large_err)]
pub(super) fn parse_request_symbol_list(
    raw: &str,
) -> Result<Vec<String>, axum::response::Response> {
    let mut symbols = Vec::new();
    for token in raw.split(|c: char| c == ',' || c.is_whitespace()) {
        if token.trim().is_empty() {
            continue;
        }
        symbols.push(parse_request_security_code(token.to_string())?);
    }
    symbols.sort_unstable();
    symbols.dedup();
    Ok(symbols)
}

/// 解析 HTTP request 的證券代號欄位，格式錯誤時回傳一致的 400 response。
#[allow(clippy::result_large_err)]
pub(super) fn parse_request_security_code(
    security_code: String,
) -> Result<String, axum::response::Response> {
    normalize_security_code(security_code).map_err(|why| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: why.to_string(),
            }),
        )
            .into_response()
    })
}

/// 正規化並驗證證券代號。
///
/// 此函式會 trim 頭尾空白，拒絕空字串與非 ASCII 英數字元，成功時回傳清理後的代號。
pub(crate) fn normalize_security_code(security_code: String) -> Result<String> {
    // 移除表單或 API 呼叫常見的頭尾空白。
    let security_code = security_code.trim();
    // 空代號無法執行任何回補流程，直接回報輸入錯誤。
    if security_code.is_empty() {
        return Err(anyhow!("security_code is required"));
    }
    // 僅允許英數代號，避免把符號或空白帶入 crawler/database 查詢。
    if !security_code.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Err(anyhow!(
            "security_code may only contain ASCII letters or numbers"
        ));
    }
    // 回傳擁有權字串，讓後續 async job 可以安全 move 進背景 task。
    Ok(security_code.to_string())
}

#[cfg(test)]
mod tests;
