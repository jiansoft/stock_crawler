//! `/portfolio/dividend-forecast/accuracy` handler：把某次股利預估快照與實際入帳比對。
//!
//! 快照與入帳由 [`crate::infra::database::repository::portfolio::forecast::load_accuracy_inputs`]
//! 讀取，比對規則在 [`crate::domain::portfolio::forecast_accuracy`]。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::Local;

use super::{error_response, parse_optional_date, repository_error};
use crate::domain::portfolio::forecast_accuracy::{self, ForecastAccuracy};
use crate::infra::database::repository::portfolio::forecast;
use crate::interfaces::web::data_api::dto::{
    DividendForecastAccuracyParams, DividendForecastAccuracyResponse, ErrorBody,
};

/// 比對某次股利預估快照（每月 1 日 08:15）與快照後的實際入帳，依預估依據彙總誤差。
///
/// 依（代號, 發放月份）配對：有預估也有入帳為 `matched`、發放日還沒到為 `pending`、
/// 發放月份已過仍沒入帳為 `missed`、沒有預估的入帳為 `unforecast`。只含現金股利。
///
/// # Errors
///
/// 快照日格式錯誤或 `member_id` 為負數回 422、驗證失敗回 401；資料庫查詢失敗時
/// 記錄內部錯誤並回不含 SQL 細節的 500。還沒有任何快照或指定日沒有快照時回 200 與空清單。
#[utoipa::path(get, path = "/api/v1/portfolio/dividend-forecast/accuracy", tag = "data-api", params(DividendForecastAccuracyParams), responses((status = 200, body = DividendForecastAccuracyResponse), (status = 401, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn dividend_forecast_accuracy(
    Query(params): Query<DividendForecastAccuracyParams>,
) -> Response {
    let member_id = params.member_id.unwrap_or(0);
    if member_id < 0 {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "member_id 不可為負數");
    }
    let requested = match parse_optional_date(params.snapshot_date.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let snapshot_date = match requested {
        Some(date) => Some(date),
        None => match forecast::latest_snapshot_date().await {
            Ok(date) => date,
            Err(error) => return repository_error(error),
        },
    };
    let today = Local::now().date_naive();
    let accuracy = match snapshot_date {
        Some(date) => match forecast::load_accuracy_inputs(date, member_id, today).await {
            Ok(inputs) => forecast_accuracy::compare(&inputs.forecasts, &inputs.actuals, today),
            Err(error) => return repository_error(error),
        },
        None => ForecastAccuracy::default(),
    };
    Json(DividendForecastAccuracyResponse::new(
        member_id,
        snapshot_date.map(|date| date.to_string()),
        today.to_string(),
        accuracy,
    ))
    .into_response()
}
