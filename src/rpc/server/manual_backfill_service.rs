//! Manual backfill gRPC service implementation.
//!
//! 這個模組把 gRPC manual backfill API 轉接到 `web::backfill_admin` 的共用
//! job 執行器，讓 HTTP UI 和 gRPC client 看到同一份 job 狀態。

use anyhow::Result;
use chrono::NaiveDate;
use tonic::{Request, Response, Status};

use crate::{
    rpc::manual_backfill::{
        manual_backfill_server::ManualBackfill, BackfillJob, BackfillJobResponse,
        ClosingAggregateRequest, DailyQuotesRequest, GetJobRequest, ListJobsRequest,
        ListJobsResponse, SecurityCodeRequest, TaiwanStockIndexRequest, YearRequest,
    },
    web,
};

/// Manual backfill gRPC service implementation.
///
/// 服務本身不保存狀態；所有 job 狀態都委派給 `web::backfill_admin` 的全域
/// job store，因此 Web UI 與 gRPC API 可互相查詢對方建立的 job。
#[derive(Default)]
pub struct ManualBackfillService {}

#[tonic::async_trait]
impl ManualBackfill for ManualBackfillService {
    /// 建立指定交易日的各股每日收盤報價回補 job。
    async fn start_daily_quotes(
        &self,
        req: Request<DailyQuotesRequest>,
    ) -> Result<Response<BackfillJobResponse>, Status> {
        // gRPC 輸入仍使用字串，因此先解析並把格式錯誤轉成 INVALID_ARGUMENT。
        let date = parse_grpc_date(&req.into_inner().date)?;

        // 建立共用背景 job，讓 HTTP UI 也能查到這筆 gRPC 建立的工作。
        let job = web::backfill_admin::start_daily_quotes_job(date).await;
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
        let job = web::backfill_admin::start_closing_aggregate_job(date).await;
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
        let job = web::backfill_admin::start_taiwan_stock_index_job(date).await;
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
        let job = web::backfill_admin::start_received_dividend_records_job(security_code).await;
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
        let job = web::backfill_admin::start_historical_dividends_job(security_code).await;
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
        let job = web::backfill_admin::start_multiple_dividend_historical_dividends_job(year).await;
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

/// 解析 gRPC request 的日期欄位，格式錯誤時回傳 INVALID_ARGUMENT。
fn parse_grpc_date(date: &str) -> Result<NaiveDate, Status> {
    NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d")
        .map_err(|why| Status::invalid_argument(format!("date must use YYYY-MM-DD: {why}")))
}

/// 解析 gRPC request 的證券代號欄位，格式錯誤時回傳 INVALID_ARGUMENT。
fn parse_grpc_security_code(security_code: String) -> Result<String, Status> {
    web::backfill_admin::normalize_security_code(security_code)
        .map_err(|why| Status::invalid_argument(why.to_string()))
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
