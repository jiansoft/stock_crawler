//! Manual backfill HTTP API 的請求解析與錯誤回應：把 request 欄位轉成 job 參數，的測試。

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
