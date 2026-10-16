//! `/market/regime` handler：加權指數（TAIEX）的趨勢狀態。
//!
//! 一次讀資料日之前最近 [`regime::LOOKBACK`] 個交易日的收盤，以
//! [`crate::domain::market_index::regime`] 計算 5／20／60／120／240 日均線的位置與方向、均線排列、
//! 站在 60 日均線同一側的天數、52 週高低點距離與近 20／60／240 日漲跌幅，用來判斷多空環境
//! （條件選股回測的 `market_filter_ma` 用的也是加權指數相對均線的位置）。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use super::{error_response, parse_optional_date, repository_error, round2};
use crate::domain::market_index::regime::{self, MarketRegime};
use crate::infra::database::repository::market_index::PgMarketIndexRepository;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, MarketRegimeParams, MarketRegimeResponse, MovingAverageStatus,
};

/// 查詢加權指數的趨勢狀態。
///
/// # Errors
///
/// 日期格式錯誤回 422；資料日之前沒有加權指數回 404；驗證失敗回 401；資料庫查詢失敗回不含
/// SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/regime", tag = "data-api", params(MarketRegimeParams), responses((status = 200, body = MarketRegimeResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn market_regime(Query(params): Query<MarketRegimeParams>) -> Response {
    let date = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let points = match regime::load_taiex(&PgMarketIndexRepository::new(), date).await {
        Ok(points) => points,
        Err(error) => return repository_error(error),
    };
    match regime::regime(&points) {
        Some(state) => Json(response(&state)).into_response(),
        None => error_response(StatusCode::NOT_FOUND, "查無加權指數資料"),
    }
}

/// 把趨勢狀態轉成回應；數值四捨五入到小數兩位。
fn response(state: &MarketRegime) -> MarketRegimeResponse {
    MarketRegimeResponse {
        data_as_of: state.date.to_string(),
        close: round2(state.close),
        change: state.change.map(round2),
        change_percent: state.change_percent.map(round2),
        alignment: state.alignment.as_str().to_owned(),
        moving_averages: state
            .moving_averages
            .iter()
            .map(|status| MovingAverageStatus {
                days: status.days,
                value: status.value.map(round2),
                distance_percent: status.distance_percent.map(round2),
                above: status.above,
                rising: status.rising,
            })
            .collect(),
        days_on_side_of_ma60: state.days_on_side_of_ma60,
        high_52_week: round2(state.high_52_week),
        low_52_week: round2(state.low_52_week),
        from_high_percent: round2(state.from_high_percent),
        from_low_percent: round2(state.from_low_percent),
        return_20d: state.return_20d.map(round2),
        return_60d: state.return_60d.map(round2),
        return_240d: state.return_240d.map(round2),
    }
}

#[cfg(test)]
mod tests;
