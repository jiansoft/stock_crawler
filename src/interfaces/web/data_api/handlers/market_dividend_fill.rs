//! `/market/dividend-fill-stats` handler：全市場依現金殖利率分組的填息率與填息天數。
//!
//! 樣本來自 [`PgDividendFillRepository`] 並經 [`dividend_fill_stats`] 快取 6 小時；
//! 持股填息的同組基準（`peer`）共用同一份樣本。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::Local;

use super::{error_response, repository_error};
use crate::domain::dividend::fill_stats::{
    BucketStats, MATURE_TRADING_DAYS, bucket_stats, stats_window,
};
use crate::infra::cache::dividend_fill_stats;
use crate::infra::database::repository::dividend_fill::PgDividendFillRepository;
use crate::interfaces::web::data_api::dto::{
    DividendFillBucket, ErrorBody, MarketDividendFillStatsParams, MarketDividendFillStatsResponse,
};

/// 查詢全市場填息基準。
///
/// # Errors
///
/// `years` 超出 1–8 回 422；驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/dividend-fill-stats", tag = "data-api", params(MarketDividendFillStatsParams), responses((status = 200, body = MarketDividendFillStatsResponse), (status = 401, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn market_dividend_fill_stats(
    Query(params): Query<MarketDividendFillStatsParams>,
) -> Response {
    let years = params.years.unwrap_or(5);
    if !(1..=8).contains(&years) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "years 必須介於 1 至 8");
    }
    let today = Local::now().date_naive();
    let samples = match dividend_fill_stats::market_fill_samples_for(
        &PgDividendFillRepository::new(),
        today,
        years,
    )
    .await
    {
        Ok(samples) => samples,
        Err(error) => return repository_error(error),
    };
    let (from, to) = stats_window(today, years);
    Json(MarketDividendFillStatsResponse {
        years,
        from: from.to_string(),
        to: to.to_string(),
        mature_trading_days: MATURE_TRADING_DAYS,
        buckets: bucket_stats(&samples).iter().map(bucket_dto).collect(),
    })
    .into_response()
}

/// 領域的分組統計轉成回應格式；持股填息的 `peer` 也用它。
pub(super) fn bucket_dto(stats: &BucketStats) -> DividendFillBucket {
    DividendFillBucket {
        bucket: stats.bucket.code().to_owned(),
        label: stats.bucket.label().to_owned(),
        events: stats.events,
        mature_events: stats.mature_events,
        fill_rate_20d: stats.fill_rate_20,
        fill_rate_60d: stats.fill_rate_60,
        fill_rate_120d: stats.fill_rate_120,
        median_trading_days_to_fill: stats.median_trading_days,
    }
}
