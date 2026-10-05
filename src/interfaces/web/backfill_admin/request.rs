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
mod tests {
    use super::super::state::MAX_CONCURRENT_JOBS;
    use super::*;
    use axum::http::StatusCode;
    use rust_decimal_macros::dec;

    /// 取出解析失敗時回傳的 HTTP 狀態碼。
    fn status_of<T>(result: Result<T, axum::response::Response>) -> StatusCode {
        result
            .err()
            .map(|response| response.status())
            .expect("應為解析失敗")
    }

    /// 日期需為 `YYYY-MM-DD`；解析失敗一律回 400。
    #[test]
    fn request_date_accepts_iso_dates_and_rejects_the_rest() {
        assert_eq!(
            parse_request_date(" 2026-04-30 ").expect("合法日期"),
            NaiveDate::from_ymd_opt(2026, 4, 30).expect("測試日期應合法")
        );
        for raw in ["", "2026-04", "2026/04/30", "not-a-date", "2026-02-30"] {
            assert_eq!(status_of(parse_request_date(raw)), StatusCode::BAD_REQUEST);
        }
    }

    /// `<input type="month">` 送出的 `YYYY-MM` 應解析成該月一日。
    #[test]
    fn request_month_resolves_to_the_first_day() {
        assert_eq!(
            parse_request_month(" 2015-01 ").expect("合法月份"),
            NaiveDate::from_ymd_opt(2015, 1, 1).expect("測試日期應合法")
        );
        for raw in ["", "2015", "2015-13", "2015-01-01"] {
            assert_eq!(status_of(parse_request_month(raw)), StatusCode::BAD_REQUEST);
        }
    }

    /// 期間代碼必須對得上 `CagrPeriod`，未知代碼在建立 job 前就擋下。
    #[test]
    fn request_period_only_accepts_known_codes() {
        let period = parse_request_period(" Y1 ").expect("Y1 應為合法期間");
        assert_eq!(period.code(), "Y1");
        for raw in ["", "Y99", "1Y"] {
            assert_eq!(
                status_of(parse_request_period(raw)),
                StatusCode::BAD_REQUEST
            );
        }
    }

    /// 股數變動比例必須為正數且不得離譜；邊界值 1000 仍屬合法。
    #[test]
    fn request_share_ratio_guards_against_nonsense_values() {
        assert_eq!(parse_request_share_ratio(" 4 ").expect("正整數"), dec!(4));
        assert_eq!(
            parse_request_share_ratio("0.5").expect("小數比例"),
            dec!(0.5)
        );
        assert_eq!(
            parse_request_share_ratio("1000").expect("上界值應合法"),
            dec!(1000)
        );
        // 0 會讓持股歸零、負數無意義、`1:4` 是把整串貼進來的常見錯誤。
        for raw in ["0", "-4", "1:4", "", "1000.1"] {
            assert_eq!(
                status_of(parse_request_share_ratio(raw)),
                StatusCode::BAD_REQUEST,
                "share_ratio={raw} 應被拒絕"
            );
        }
    }

    /// 代號清單支援逗號／空白／換行混用，並排序去重；空字串代表「全部 ETF」。
    #[test]
    fn request_symbol_list_normalizes_separators_and_duplicates() {
        assert_eq!(
            parse_request_symbol_list("0056, 0050\n0056\t2330").expect("合法清單"),
            vec!["0050".to_string(), "0056".to_string(), "2330".to_string()]
        );
        assert!(
            parse_request_symbol_list("  \n ")
                .expect("空白視為空清單")
                .is_empty()
        );
        // 只要有一個代號非法，整份清單就該被拒絕，不能只丟掉那一個。
        assert_eq!(
            status_of(parse_request_symbol_list("0050, 00;50")),
            StatusCode::BAD_REQUEST
        );
    }

    /// 證券代號只接受 ASCII 英數，並去除頭尾空白。
    #[test]
    fn security_code_is_trimmed_and_restricted_to_ascii_alphanumerics() {
        assert_eq!(
            normalize_security_code(" 2330 ".to_string()).expect("合法代號"),
            "2330"
        );
        // trim 只作用於頭尾，中間的空白仍屬非法字元。
        assert_eq!(
            normalize_security_code("2330\n".to_string()).expect("尾端換行應被 trim"),
            "2330"
        );
        for raw in ["", "   ", "23 30", "0050;DROP", "台積電"] {
            assert!(
                normalize_security_code(raw.to_string()).is_err(),
                "security_code={raw:?} 應被拒絕"
            );
        }
        assert_eq!(
            status_of(parse_request_security_code("0050;".to_string())),
            StatusCode::BAD_REQUEST
        );
    }

    /// 建立被拒絕的兩種原因各自對應固定的 HTTP 狀態碼。
    #[test]
    fn start_job_errors_map_to_conflict_and_too_many_requests() {
        let duplicate = StartJobError::DuplicateActiveJob {
            existing_id: "20260815000000-1".to_string(),
        };
        assert!(duplicate.to_string().contains("20260815000000-1"));
        assert_eq!(
            start_job_error_response(duplicate).status(),
            StatusCode::CONFLICT
        );

        let too_many = StartJobError::TooManyActiveJobs {
            max: MAX_CONCURRENT_JOBS,
        };
        assert!(
            too_many
                .to_string()
                .contains(&MAX_CONCURRENT_JOBS.to_string())
        );
        assert_eq!(
            start_job_error_response(too_many).status(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
}
