use chrono::NaiveDate;
use rust_decimal::Decimal;
use tonic::Code;

use super::*;

/// 公司行動比例的 gRPC 驗證規則應與 HTTP API 一致。
#[test]
fn corporate_action_share_ratio_validation_rejects_invalid_values() {
    assert_eq!(
        parse_grpc_share_ratio("4").expect("正整數比例應合法"),
        Decimal::from(4)
    );
    for raw in ["0", "-1", "1:4", "1001"] {
        let err = parse_grpc_share_ratio(raw).expect_err("非法比例應被拒絕");
        assert_eq!(err.code(), Code::InvalidArgument, "raw={raw}");
    }
}

/// CAGR 期間 gRPC 在建立 job 前就應拒絕未知代碼。
#[tokio::test]
async fn start_cagr_period_rejects_unknown_period_before_job_creation() {
    let service = ManualBackfillServiceImpl::default();
    let err = service
        .start_cagr_period(Request::new(CagrPeriodRequest {
            period: "Y8".to_string(),
        }))
        .await
        .expect_err("未知期間應失敗");
    assert_eq!(err.code(), Code::InvalidArgument);
}

/// Corporate Action gRPC 應在確認比例合法後才觸碰資料庫。
#[tokio::test]
async fn save_corporate_action_rejects_invalid_ratio_before_database_write() {
    let service = ManualBackfillServiceImpl::default();
    let err = service
        .save_corporate_action(Request::new(CorporateActionRequest {
            stock_symbol: "0050".to_string(),
            effective_date: "2025-06-18".to_string(),
            share_ratio: "0".to_string(),
            note: String::new(),
        }))
        .await
        .expect_err("零比例應在寫入前被拒絕");
    assert_eq!(err.code(), Code::InvalidArgument);
}

/// Quote History gRPC 應在建立 job 前驗證月份與代號格式。
#[tokio::test]
async fn start_quote_history_rejects_invalid_input_before_job_creation() {
    let service = ManualBackfillServiceImpl::default();
    let invalid_month = service
        .start_quote_history(Request::new(QuoteHistoryRequest {
            stock_symbols: "0050".to_string(),
            from: "2015".to_string(),
            to: "2021-12".to_string(),
        }))
        .await
        .expect_err("無效月份應失敗");
    assert_eq!(invalid_month.code(), Code::InvalidArgument);

    let invalid_symbol = service
        .start_quote_history(Request::new(QuoteHistoryRequest {
            stock_symbols: "0050; DROP".to_string(),
            from: "2015-01".to_string(),
            to: "2021-12".to_string(),
        }))
        .await
        .expect_err("無效代號應失敗");
    assert_eq!(invalid_symbol.code(), Code::InvalidArgument);
}

/// 建立 job 被拒絕的兩種原因各自對應固定的 gRPC 狀態碼（與 HTTP 409/429 對齊）。
#[test]
fn start_job_errors_map_to_grpc_status_codes() {
    let duplicate = web::backfill_admin::StartJobError::DuplicateActiveJob {
        existing_id: "20260815000000-1".to_string(),
    };
    let status = start_job_error_to_status(duplicate);
    assert_eq!(status.code(), Code::AlreadyExists);
    assert!(status.message().contains("20260815000000-1"));

    let too_many = web::backfill_admin::StartJobError::TooManyActiveJobs { max: 4 };
    assert_eq!(
        start_job_error_to_status(too_many).code(),
        Code::ResourceExhausted
    );
}

/// 日期欄位僅接受 `YYYY-MM-DD`，其餘一律 INVALID_ARGUMENT。
#[test]
fn grpc_date_parsing_matches_the_http_rules() {
    assert_eq!(
        parse_grpc_date(" 2026-04-30 ").expect("合法日期"),
        NaiveDate::from_ymd_opt(2026, 4, 30).expect("測試日期應合法")
    );
    for raw in ["", "2026-04", "2026/04/30", "2026-02-30"] {
        let err = parse_grpc_date(raw).expect_err("非法日期應被拒絕");
        assert_eq!(err.code(), Code::InvalidArgument, "raw={raw}");
    }
}

/// 月份欄位為 `YYYY-MM`，解析結果落在該月一日。
#[test]
fn grpc_month_parsing_resolves_to_the_first_day() {
    assert_eq!(
        parse_grpc_month(" 2015-01 ").expect("合法月份"),
        NaiveDate::from_ymd_opt(2015, 1, 1).expect("測試日期應合法")
    );
    for raw in ["", "2015", "2015-13", "2015-01-01"] {
        let err = parse_grpc_month(raw).expect_err("非法月份應被拒絕");
        assert_eq!(err.code(), Code::InvalidArgument, "raw={raw}");
    }
}

/// 代號驗證共用 HTTP 端規則：trim 後只允許 ASCII 英數。
#[test]
fn grpc_security_code_shares_the_http_validation() {
    assert_eq!(
        parse_grpc_security_code(" 2330 ".to_string()).expect("合法代號"),
        "2330"
    );
    for raw in ["", "23 30", "0050;DROP"] {
        let err = parse_grpc_security_code(raw.to_string()).expect_err("非法代號應被拒絕");
        assert_eq!(err.code(), Code::InvalidArgument, "raw={raw}");
    }
}

/// 查無此 job 時回傳 NOT_FOUND，訊息帶上查詢的 id。
#[tokio::test]
async fn get_job_returns_not_found_for_an_unknown_id() {
    let service = ManualBackfillServiceImpl::default();
    let err = service
        .get_job(Request::new(GetJobRequest {
            id: "no-such-job".to_string(),
        }))
        .await
        .expect_err("未知 id 應回 NOT_FOUND");

    assert_eq!(err.code(), Code::NotFound);
    assert!(err.message().contains("no-such-job"));
}

/// 年度明顯超出合理範圍時，在建立 job 前就拒絕。
#[tokio::test]
async fn multiple_dividend_backfill_rejects_out_of_range_years() {
    let service = ManualBackfillServiceImpl::default();
    for year in [1899, 3001, 0, -1] {
        let err = service
            .start_multiple_dividend_historical_dividends(Request::new(YearRequest { year }))
            .await
            .expect_err("超出範圍的年度應被拒絕");
        assert_eq!(err.code(), Code::InvalidArgument, "year={year}");
    }
}

/// 各 start API 都應在觸碰資料庫或外部網站前先驗證日期格式。
#[tokio::test]
async fn start_apis_validate_dates_before_creating_jobs() {
    let service = ManualBackfillServiceImpl::default();

    let daily = service
        .start_daily_quotes(Request::new(DailyQuotesRequest {
            date: "2026-04".to_string(),
        }))
        .await
        .expect_err("非法日期應失敗");
    assert_eq!(daily.code(), Code::InvalidArgument);

    let closing = service
        .start_closing_aggregate(Request::new(ClosingAggregateRequest {
            date: "not-a-date".to_string(),
        }))
        .await
        .expect_err("非法日期應失敗");
    assert_eq!(closing.code(), Code::InvalidArgument);

    let index = service
        .start_taiwan_stock_index(Request::new(TaiwanStockIndexRequest {
            date: String::new(),
        }))
        .await
        .expect_err("空日期應失敗");
    assert_eq!(index.code(), Code::InvalidArgument);

    // CAGR 允許留空（採最新交易日），但填了就必須合法。
    let cagr = service
        .start_cagr(Request::new(CagrRequest {
            date: "not-a-date".to_string(),
        }))
        .await
        .expect_err("非法日期應失敗");
    assert_eq!(cagr.code(), Code::InvalidArgument);
}

/// 代號類 API 在建立 job 前先驗證代號。
#[tokio::test]
async fn start_apis_validate_security_codes_before_creating_jobs() {
    let service = ManualBackfillServiceImpl::default();

    let received = service
        .start_received_dividend_records(Request::new(SecurityCodeRequest {
            security_code: "0050; DROP".to_string(),
        }))
        .await
        .expect_err("非法代號應失敗");
    assert_eq!(received.code(), Code::InvalidArgument);

    let historical = service
        .start_historical_dividends(Request::new(SecurityCodeRequest {
            security_code: String::new(),
        }))
        .await
        .expect_err("空代號應失敗");
    assert_eq!(historical.code(), Code::InvalidArgument);
}

/// Quote History 代號清單應支援混合分隔、排序及去重。
#[test]
fn quote_history_symbol_list_is_normalized() {
    assert_eq!(
        parse_grpc_symbol_list("0056, 0050\n0056").expect("代號清單應合法"),
        vec!["0050".to_string(), "0056".to_string()]
    );
    assert!(
        parse_grpc_symbol_list("  ")
            .expect("空清單應合法")
            .is_empty()
    );
}

/// job 轉成 gRPC message：狀態用 snake_case 字串，未完成時 finished_at 為空字串。
#[test]
fn backfill_jobs_convert_to_grpc_messages() {
    let running = to_grpc_job(web::backfill_admin::BackfillJob::for_test("job-1", None));
    assert_eq!(
        (
            running.id.as_str(),
            running.kind.as_str(),
            running.status.as_str()
        ),
        ("job-1", "cagr", "running")
    );
    assert_eq!(running.input, "date=2026-10-20");
    assert_eq!(running.message, "ok");
    assert_eq!(running.started_at, "2026-10-20T10:00:00+08:00");
    assert!(running.finished_at.is_empty());

    let done = to_grpc_job(web::backfill_admin::BackfillJob::for_test(
        "job-2",
        Some("2026-10-20T10:05:00+08:00"),
    ));
    assert_eq!(done.status, "succeeded");
    assert_eq!(done.finished_at, "2026-10-20T10:05:00+08:00");
}
