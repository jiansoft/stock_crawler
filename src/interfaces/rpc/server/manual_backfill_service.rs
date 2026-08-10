//! Manual backfill gRPC service implementation.
//!
//! 這個模組把 gRPC manual backfill API 轉接到 `web::backfill_admin` 的共用
//! job 執行器，讓 HTTP UI 和 gRPC client 看到同一份 job 狀態。

use anyhow::Result;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use tonic::{Request, Response, Status};

use crate::{
    domain::performance::{CagrPeriod, CorporateAction, CorporateActionRepository},
    infra::database::repository::corporate_action::PgCorporateActionRepository,
    interfaces::rpc::manual_backfill::{
        BackfillJob,
        BackfillJobResponse,
        CagrPeriodRequest,
        CagrRequest,
        ClosingAggregateRequest,
        CorporateActionItem,
        CorporateActionRequest,
        CorporateActionResponse,
        DailyQuotesRequest,
        GetJobRequest,
        ListJobsRequest,
        ListJobsResponse,
        QuoteHistoryRequest,
        SecurityCodeRequest,
        TaiwanStockIndexRequest,
        YearRequest,
        // 服務定義改名為 ManualBackfillService 後，tonic 產生的 trait 也相應更名
        manual_backfill_service_server::ManualBackfillService,
    },
    interfaces::web,
};

/// Manual backfill gRPC 服務實作。
///
/// 服務本身不保存狀態；所有 job 狀態都委派給 `web::backfill_admin` 的全域
/// job store，因此 Web UI 與 gRPC API 可互相查詢對方建立的 job。
#[derive(Default)]
pub struct ManualBackfillServiceImpl {}

#[tonic::async_trait]
// 實作 tonic 產生的 ManualBackfillService trait
impl ManualBackfillService for ManualBackfillServiceImpl {
    /// 建立指定交易日的各股每日收盤報價回補 job。
    async fn start_daily_quotes(
        &self,
        req: Request<DailyQuotesRequest>,
    ) -> Result<Response<BackfillJobResponse>, Status> {
        // gRPC 輸入仍使用字串，因此先解析並把格式錯誤轉成 INVALID_ARGUMENT。
        let date = parse_grpc_date(&req.into_inner().date)?;

        // 建立共用背景 job，讓 HTTP UI 也能查到這筆 gRPC 建立的工作。
        // 建立可能被拒絕（重複 job / 併行已滿），轉成對應的 gRPC 錯誤碼。
        let job = web::backfill_admin::start_daily_quotes_job(date)
            .await
            .map_err(start_job_error_to_status)?;
        // 轉成 proto response model 後回傳。
        Ok(Response::new(BackfillJobResponse {
            job: Some(to_grpc_job(job)),
        }))
    }

    /// 建立指定交易日的收盤彙總回補 job。
    async fn start_closing_aggregate(
        &self,
        req: Request<ClosingAggregateRequest>,
    ) -> Result<Response<BackfillJobResponse>, Status> {
        // 取出 prost 產生的 request body。
        let req = req.into_inner();
        // gRPC 輸入仍使用字串，因此先解析並把格式錯誤轉成 INVALID_ARGUMENT。
        let date = parse_grpc_date(&req.date)?;

        // 建立共用背景 job，讓 HTTP UI 也能查到這筆 gRPC 建立的工作。
        // 建立可能被拒絕（重複 job / 併行已滿），轉成對應的 gRPC 錯誤碼。
        let job = web::backfill_admin::start_closing_aggregate_job(date)
            .await
            .map_err(start_job_error_to_status)?;
        // 轉成 proto response model 後回傳。
        Ok(Response::new(BackfillJobResponse {
            job: Some(to_grpc_job(job)),
        }))
    }

    /// 建立指定日期的台股加權指數回補 job。
    async fn start_taiwan_stock_index(
        &self,
        req: Request<TaiwanStockIndexRequest>,
    ) -> Result<Response<BackfillJobResponse>, Status> {
        // gRPC 輸入仍使用字串，因此先解析並把格式錯誤轉成 INVALID_ARGUMENT。
        let date = parse_grpc_date(&req.into_inner().date)?;

        // 建立共用背景 job，只會 upsert 指定日期的指數資料。
        // 建立可能被拒絕（重複 job / 併行已滿），轉成對應的 gRPC 錯誤碼。
        let job = web::backfill_admin::start_taiwan_stock_index_job(date)
            .await
            .map_err(start_job_error_to_status)?;
        Ok(Response::new(BackfillJobResponse {
            job: Some(to_grpc_job(job)),
        }))
    }

    /// 建立單一證券的持股已領股利重算 job。
    async fn start_received_dividend_records(
        &self,
        req: Request<SecurityCodeRequest>,
    ) -> Result<Response<BackfillJobResponse>, Status> {
        // 共用 Web API 的證券代號驗證規則，維持 HTTP/gRPC 行為一致。
        let security_code = parse_grpc_security_code(req.into_inner().security_code)?;

        // 建立背景 job，立即回傳 job id 與初始狀態。
        // 建立可能被拒絕（重複 job / 併行已滿），轉成對應的 gRPC 錯誤碼。
        let job = web::backfill_admin::start_received_dividend_records_job(security_code)
            .await
            .map_err(start_job_error_to_status)?;
        Ok(Response::new(BackfillJobResponse {
            job: Some(to_grpc_job(job)),
        }))
    }

    /// 建立單一證券的歷年股利補抓 job。
    async fn start_historical_dividends(
        &self,
        req: Request<SecurityCodeRequest>,
    ) -> Result<Response<BackfillJobResponse>, Status> {
        // 共用 Web API 的證券代號驗證規則，維持 HTTP/gRPC 行為一致。
        let security_code = parse_grpc_security_code(req.into_inner().security_code)?;

        // 建立背景 job，實際 Yahoo 抓取與 upsert 會在背景 task 中執行。
        // 建立可能被拒絕（重複 job / 併行已滿），轉成對應的 gRPC 錯誤碼。
        let job = web::backfill_admin::start_historical_dividends_job(security_code)
            .await
            .map_err(start_job_error_to_status)?;
        Ok(Response::new(BackfillJobResponse {
            job: Some(to_grpc_job(job)),
        }))
    }

    /// 建立指定年度多次配息股票的歷年股利批次回補 job。
    async fn start_multiple_dividend_historical_dividends(
        &self,
        req: Request<YearRequest>,
    ) -> Result<Response<BackfillJobResponse>, Status> {
        // 年度資料表查詢預期使用合理西元年，先擋掉明顯輸入錯誤。
        let year = req.into_inner().year;
        if !(1900..=3000).contains(&year) {
            return Err(Status::invalid_argument(
                "year must be between 1900 and 3000",
            ));
        }

        // 建立共用背景 job，讓 HTTP UI 也能查到這筆 gRPC 建立的工作。
        // 建立可能被拒絕（重複 job / 併行已滿），轉成對應的 gRPC 錯誤碼。
        let job = web::backfill_admin::start_multiple_dividend_historical_dividends_job(year)
            .await
            .map_err(start_job_error_to_status)?;
        Ok(Response::new(BackfillJobResponse {
            job: Some(to_grpc_job(job)),
        }))
    }

    /// 建立歷史日報價回補 job。
    ///
    /// 代號可以逗號或空白分隔；留空代表全部未下市 ETF／ETN。月份區間含首尾。
    async fn start_quote_history(
        &self,
        req: Request<QuoteHistoryRequest>,
    ) -> Result<Response<BackfillJobResponse>, Status> {
        let req = req.into_inner();
        let from = parse_grpc_month(&req.from)?;
        let to = parse_grpc_month(&req.to)?;
        let stock_symbols = parse_grpc_symbol_list(&req.stock_symbols)?;
        let job = web::backfill_admin::start_quote_history_job(stock_symbols, from, to)
            .await
            .map_err(start_job_error_to_status)?;

        Ok(Response::new(BackfillJobResponse {
            job: Some(to_grpc_job(job)),
        }))
    }

    /// 新增或更新一筆公司行動，並回傳該股全部已登錄事件。
    ///
    /// 公司行動是同步小型寫入，不建立背景 job；回應可讓維運人員立即核對。
    async fn save_corporate_action(
        &self,
        req: Request<CorporateActionRequest>,
    ) -> Result<Response<CorporateActionResponse>, Status> {
        let req = req.into_inner();
        let stock_symbol = parse_grpc_security_code(req.stock_symbol)?;
        let effective_date = parse_grpc_date(&req.effective_date)?;
        let share_ratio = parse_grpc_share_ratio(&req.share_ratio)?;
        let repository = PgCorporateActionRepository::new();
        let action = CorporateAction {
            stock_symbol: stock_symbol.clone(),
            effective_date,
            share_ratio,
            note: req.note.trim().to_owned(),
        };

        repository
            .save(&action)
            .await
            .map_err(|why| Status::internal(format!("failed to save corporate action: {why:#}")))?;
        let actions = repository
            .fetch_by_symbol(&stock_symbol)
            .await
            .map_err(|why| {
                Status::internal(format!("failed to read back corporate actions: {why:#}"))
            })?
            .into_iter()
            .map(|item| CorporateActionItem {
                effective_date: item.effective_date.to_string(),
                share_ratio: item.share_ratio.normalize().to_string(),
                note: item.note,
            })
            .collect();

        Ok(Response::new(CorporateActionResponse {
            stock_symbol,
            actions,
        }))
    }

    /// 建立指定基準日的 CAGR 重算 job。
    ///
    /// 日期留空時使用資料庫最新交易日，與 HTTP admin API 的語義一致。
    async fn start_cagr(
        &self,
        req: Request<CagrRequest>,
    ) -> Result<Response<BackfillJobResponse>, Status> {
        let raw_date = req.into_inner().date;
        let date = if raw_date.trim().is_empty() {
            None
        } else {
            Some(parse_grpc_date(&raw_date)?)
        };
        let job = web::backfill_admin::start_cagr_job(date)
            .await
            .map_err(start_job_error_to_status)?;

        Ok(Response::new(BackfillJobResponse {
            job: Some(to_grpc_job(job)),
        }))
    }

    /// 建立單一 CAGR 統計期間的歷史基準日回填 job。
    async fn start_cagr_period(
        &self,
        req: Request<CagrPeriodRequest>,
    ) -> Result<Response<BackfillJobResponse>, Status> {
        let raw_period = req.into_inner().period;
        let period = CagrPeriod::from_code(raw_period.trim())
            .ok_or_else(|| Status::invalid_argument(format!("unknown period: {raw_period}")))?;
        let job = web::backfill_admin::start_cagr_period_job(period)
            .await
            .map_err(start_job_error_to_status)?;

        Ok(Response::new(BackfillJobResponse {
            job: Some(to_grpc_job(job)),
        }))
    }

    /// 列出目前程序內所有 manual backfill jobs。
    async fn list_jobs(
        &self,
        _req: Request<ListJobsRequest>,
    ) -> Result<Response<ListJobsResponse>, Status> {
        // 讀取共用 job store，並逐筆轉成 gRPC 產生碼使用的 message 型別。
        let jobs = web::backfill_admin::list_backfill_jobs()
            .await
            .into_iter()
            .map(to_grpc_job)
            .collect();

        Ok(Response::new(ListJobsResponse { jobs }))
    }

    /// 依 job id 查詢單一 manual backfill job。
    async fn get_job(
        &self,
        req: Request<GetJobRequest>,
    ) -> Result<Response<BackfillJobResponse>, Status> {
        // gRPC request 只帶 id，查不到時回傳 NOT_FOUND。
        let id = req.into_inner().id;
        let job = web::backfill_admin::get_backfill_job(&id)
            .await
            .ok_or_else(|| Status::not_found(format!("job not found: {id}")))?;

        // 查到時包成 BackfillJobResponse，對齊 start job API 的回應格式。
        Ok(Response::new(BackfillJobResponse {
            job: Some(to_grpc_job(job)),
        }))
    }
}

/// 將建立 job 被拒絕的原因轉成對應的 gRPC 狀態碼。
///
/// - 相同 job 執行中 → `ALREADY_EXISTS`（對應 HTTP 409）。
/// - 併行名額已滿 → `RESOURCE_EXHAUSTED`（對應 HTTP 429）。
fn start_job_error_to_status(err: web::backfill_admin::StartJobError) -> Status {
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
fn parse_grpc_date(date: &str) -> Result<NaiveDate, Status> {
    NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d")
        .map_err(|why| Status::invalid_argument(format!("date must use YYYY-MM-DD: {why}")))
}

/// 解析 gRPC request 的月份欄位（`YYYY-MM`），並回傳該月一日。
fn parse_grpc_month(month: &str) -> Result<NaiveDate, Status> {
    NaiveDate::parse_from_str(&format!("{}-01", month.trim()), "%Y-%m-%d")
        .map_err(|why| Status::invalid_argument(format!("month must use YYYY-MM: {why}")))
}

/// 解析逗號或空白分隔的證券代號，並排序去重。
///
/// 空字串保留為空清單，由 job runner 解釋為全部 ETF／ETN。
fn parse_grpc_symbol_list(raw: &str) -> Result<Vec<String>, Status> {
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
fn parse_grpc_security_code(security_code: String) -> Result<String, Status> {
    web::backfill_admin::normalize_security_code(security_code)
        .map_err(|why| Status::invalid_argument(why.to_string()))
}

/// 解析 gRPC 公司行動股數比例，並擋下零、負數與明顯異常的大值。
fn parse_grpc_share_ratio(raw: &str) -> Result<Decimal, Status> {
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
fn to_grpc_job(job: web::backfill_admin::BackfillJob) -> BackfillJob {
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

#[cfg(test)]
mod tests {
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
}
