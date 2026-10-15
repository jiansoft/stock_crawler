//! `/market/regime` handler：加權指數（TAIEX）的趨勢狀態。
//!
//! 一次讀資料日之前最近 [`LOOKBACK`] 個交易日的收盤，計算 5／20／60／120／240 日均線的位置與
//! 方向、均線排列、站在 60 日均線同一側的天數、52 週高低點距離與近 20／60／240 日漲跌幅，
//! 用來判斷多空環境（條件選股回測的 `market_filter_ma` 用的也是加權指數相對均線的位置）。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;

use super::{database_error, error_response, parse_optional_date, round2};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, MarketRegimeParams, MarketRegimeResponse, MovingAverageStatus,
};

/// 均線天數。
const MA_DAYS: [u16; 5] = [5, 20, 60, 120, 240];

/// 判斷均線方向時與幾個交易日前的均線比較。
const SLOPE_LAG: usize = 5;

/// 52 週約幾個交易日。
const WEEKS_52_DAYS: usize = 250;

/// 讀取的交易日數：240 日均線往回數 [`SLOPE_LAG`] 天，以及站在 60 日均線同一側的天數都夠用。
const LOOKBACK: i64 = 520;

/// `$1`（含）之前最近 `$2` 個交易日的加權指數收盤，由新到舊；`$1` 為 `NULL` 時從最新一天起。
const TAIEX_SQL: &str = r#"
SELECT "date", "index"::float8
FROM "index"
WHERE category = 'TAIEX' AND ($1::date IS NULL OR "date" <= $1)
ORDER BY "date" DESC
LIMIT $2
"#;

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
    let points: Vec<(NaiveDate, f64)> = match sqlx::query_as(TAIEX_SQL)
        .bind(date)
        .bind(LOOKBACK)
        .fetch_all(database::get_connection())
        .await
    {
        Ok(points) => points,
        Err(error) => return database_error(error),
    };
    match regime(&points) {
        Some(response) => Json(response).into_response(),
        None => error_response(StatusCode::NOT_FOUND, "查無加權指數資料"),
    }
}

/// 由新到舊的 `(日期, 收盤)` 算出趨勢狀態；沒有任何資料回 `None`。
fn regime(points: &[(NaiveDate, f64)]) -> Option<MarketRegimeResponse> {
    let &(date, close) = points.first()?;
    let closes: Vec<f64> = points.iter().map(|(_, close)| *close).collect();
    let percent = |from: f64| (from > 0.0).then(|| round2((close / from - 1.0) * 100.0));

    let moving_averages: Vec<MovingAverageStatus> = MA_DAYS
        .iter()
        .map(|&days| {
            let value = moving_average(&closes, usize::from(days), 0);
            let earlier = moving_average(&closes, usize::from(days), SLOPE_LAG);
            MovingAverageStatus {
                days,
                value: value.map(round2),
                distance_percent: value.and_then(percent),
                above: value.map(|value| close >= value),
                rising: value.zip(earlier).map(|(now, before)| now > before),
            }
        })
        .collect();
    let average = |days: u16| {
        moving_averages
            .iter()
            .find(|status| status.days == days)
            .and_then(|status| status.value)
    };
    let alignment = alignment(
        close,
        [average(20), average(60), average(120), average(240)],
    );

    let window = &closes[..closes.len().min(WEEKS_52_DAYS)];
    let high = window.iter().copied().fold(f64::MIN, f64::max);
    let low = window.iter().copied().fold(f64::MAX, f64::min);
    let previous = closes.get(1).copied();

    Some(MarketRegimeResponse {
        data_as_of: date.to_string(),
        close: round2(close),
        change: previous.map(|previous| round2(close - previous)),
        change_percent: previous.and_then(percent),
        alignment: alignment.to_owned(),
        moving_averages,
        days_on_side_of_ma60: days_on_side(&closes, 60),
        high_52_week: round2(high),
        low_52_week: round2(low),
        from_high_percent: percent(high).unwrap_or(0.0),
        from_low_percent: percent(low).unwrap_or(0.0),
        return_20d: closes.get(20).copied().and_then(percent),
        return_60d: closes.get(60).copied().and_then(percent),
        return_240d: closes.get(240).copied().and_then(percent),
    })
}

/// 由新到舊的收盤從第 `offset` 個起算的 `days` 日平均；資料不足回 `None`。
fn moving_average(closes: &[f64], days: usize, offset: usize) -> Option<f64> {
    let window = closes.get(offset..offset + days)?;
    Some(window.iter().sum::<f64>() / days as f64)
}

/// 均線排列：收盤與 20／60／120／240 日均線由高到低為多頭、由低到高為空頭，其他（含資料不足）為混合。
fn alignment(close: f64, averages: [Option<f64>; 4]) -> &'static str {
    let Some(values) = averages
        .into_iter()
        .collect::<Option<Vec<f64>>>()
        .map(|averages| std::iter::once(close).chain(averages).collect::<Vec<f64>>())
    else {
        return "mixed";
    };
    if values.windows(2).all(|pair| pair[0] > pair[1]) {
        "bullish"
    } else if values.windows(2).all(|pair| pair[0] < pair[1]) {
        "bearish"
    } else {
        "mixed"
    }
}

/// 收盤連續站在 `days` 日均線同一側的交易日數（含最新一天）；最新一天沒有均線回 `None`。
fn days_on_side(closes: &[f64], days: usize) -> Option<i64> {
    let side = |offset: usize| {
        moving_average(closes, days, offset).map(|average| closes[offset] >= average)
    };
    let today = side(0)?;
    let count = (0..closes.len())
        .map_while(side)
        .take_while(|above| *above == today)
        .count();
    Some(count as i64)
}

#[cfg(test)]
mod tests;
