//! gRPC 請求欄位的解析與 job 回應的轉換；驗證規則與 HTTP 手動回補 API 共用。

use chrono::NaiveDate;
use rust_decimal::Decimal;
use tonic::Status;

use crate::interfaces::{rpc::manual_backfill::BackfillJob, web};

/// 將建立 job 被拒絕的原因轉成對應的 gRPC 狀態碼。
///
/// - 相同 job 執行中 → `ALREADY_EXISTS`（對應 HTTP 409）。
/// - 併行名額已滿 → `RESOURCE_EXHAUSTED`（對應 HTTP 429）。
pub(super) fn start_job_error_to_status(err: web::backfill_admin::StartJobError) -> Status {
    match &err {
        web::backfill_admin::StartJobError::DuplicateActiveJob { .. } => {
            Status::already_exists(err.to_string())
        }
        web::backfill_admin::StartJobError::TooManyActiveJobs { .. } => {
            Status::resource_exhausted(err.to_string())
        }
    }
}

/// 解析 gRPC request 的日期欄位，格式錯誤時回傳 INVALID_ARGUMENT。
pub(super) fn parse_grpc_date(date: &str) -> Result<NaiveDate, Status> {
    NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d")
        .map_err(|why| Status::invalid_argument(format!("date must use YYYY-MM-DD: {why}")))
}

/// 解析 gRPC request 的月份欄位（`YYYY-MM`），並回傳該月一日。
pub(super) fn parse_grpc_month(month: &str) -> Result<NaiveDate, Status> {
    NaiveDate::parse_from_str(&format!("{}-01", month.trim()), "%Y-%m-%d")
        .map_err(|why| Status::invalid_argument(format!("month must use YYYY-MM: {why}")))
}

/// 解析逗號或空白分隔的證券代號，並排序去重。
///
/// 空字串保留為空清單，由 job runner 解釋為全部 ETF／ETN。
pub(super) fn parse_grpc_symbol_list(raw: &str) -> Result<Vec<String>, Status> {
    let mut symbols = Vec::new();
    for token in raw.split(|c: char| c == ',' || c.is_whitespace()) {
        if token.trim().is_empty() {
            continue;
        }
        symbols.push(parse_grpc_security_code(token.to_string())?);
    }
    symbols.sort_unstable();
    symbols.dedup();
    Ok(symbols)
}

/// 解析 gRPC request 的證券代號欄位，格式錯誤時回傳 INVALID_ARGUMENT。
pub(super) fn parse_grpc_security_code(security_code: String) -> Result<String, Status> {
    web::backfill_admin::normalize_security_code(security_code)
        .map_err(|why| Status::invalid_argument(why.to_string()))
}

/// 解析 gRPC 公司行動股數比例，並擋下零、負數與明顯異常的大值。
pub(super) fn parse_grpc_share_ratio(raw: &str) -> Result<Decimal, Status> {
    let ratio = Decimal::from_str_exact(raw.trim()).map_err(|why| {
        Status::invalid_argument(format!("share_ratio must be a decimal number: {why}"))
    })?;
    if ratio <= Decimal::ZERO {
        return Err(Status::invalid_argument(
            "share_ratio must be greater than zero",
        ));
    }
    if ratio > Decimal::from(1000) {
        return Err(Status::invalid_argument(
            "share_ratio looks implausible (> 1000)",
        ));
    }
    Ok(ratio)
}

/// 將 crate 內部 job model 轉成 prost 產生的 gRPC job message。
pub(super) fn to_grpc_job(job: web::backfill_admin::BackfillJob) -> BackfillJob {
    // 內部狀態是 enum，gRPC message 目前以 snake_case 字串輸出。
    let status = job.status_label().to_string();

    // prost message 欄位不使用 Option<String> 表示完成時間，因此未完成時輸出空字串。
    BackfillJob {
        id: job.id,
        kind: job.kind,
        input: job.input,
        status,
        message: job.message,
        started_at: job.started_at,
        finished_at: job.finished_at.unwrap_or_default(),
    }
}
