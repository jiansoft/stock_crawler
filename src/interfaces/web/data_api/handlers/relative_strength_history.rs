//! `/stocks/{symbol}/relative-strength` handler：個股近 N 個交易日的相對強度走勢。
//!
//! 每個交易日的分數與 [`mod@super::relative_strength`] 排行同一算法：近 N 日報酬取
//! `daily_forward_return` 基準日 T−N 的 N 日欄位，加權報酬＝0.4 × 近 60 日 ＋ 0.3 × 近 120 日
//! ＋ 0.3 × 近 240 日，分數是比它弱的家數在上市櫃普通股（不含 ETF、特別股）中的百分位。每天都要以全體排名，
//! 由 [`RelativeStrengthQuery`] 一次查詢在資料庫內以視窗函式算完，只回傳這檔股票的列。

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::Local;

use super::{ensure_stock_exists, error_response, parse_optional_date, repository_error, round2};
use crate::domain::quote::relative_strength::{self, RelativeStrengthDay, RelativeStrengthQuery};
use crate::infra::database::repository::quote::PgQuoteRepository;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, RelativeStrengthCrossing, RelativeStrengthHistoryParams,
    RelativeStrengthHistoryResponse, RelativeStrengthPoint,
};

/// 查詢個股近 N 個交易日的相對強度分數、名次與穿越 50／80 的轉折。
///
/// 分數與 `/market/relative-strength` 同一算法（以當天上市櫃普通股（不含 ETF、特別股）計算）；上市未滿 240 個交易日、
/// 近 5 個交易日沒有收盤或非上市櫃的股票沒有分數，回 200 與空清單。
///
/// # Errors
///
/// 日期格式錯誤或 `days` 不在 1–120 回 422；股票不存在回 404；驗證失敗回 401；
/// 資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/relative-strength", tag = "data-api", params(("symbol" = String, Path, description = "股票代號"), RelativeStrengthHistoryParams), responses((status = 200, body = RelativeStrengthHistoryResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn relative_strength_history(
    Path(symbol): Path<String>,
    Query(params): Query<RelativeStrengthHistoryParams>,
) -> Response {
    let until = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value.unwrap_or_else(|| Local::now().date_naive()),
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let days = params.days.unwrap_or(60);
    if !(1..=120).contains(&days) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "days 必須介於 1 至 120");
    }
    if let Some(response) = ensure_stock_exists(&symbol).await {
        return response;
    }

    let rows = match PgQuoteRepository::new()
        .fetch_relative_strength(until, days, std::slice::from_ref(&symbol))
        .await
    {
        Ok(rows) => rows,
        Err(error) => return repository_error(error),
    };
    Json(build_history(symbol, days, rows)).into_response()
}

/// 由新到舊的列組成回應：分數、期間高低點（同分取最近一次）、分數變化與門檻穿越。
fn build_history(
    symbol: String,
    days: u16,
    rows: Vec<RelativeStrengthDay>,
) -> RelativeStrengthHistoryResponse {
    let items: Vec<RelativeStrengthPoint> = rows
        .into_iter()
        .map(|row| RelativeStrengthPoint {
            date: row.date.to_string(),
            rs_score: row.rs_score,
            rank: row.rank,
            universe: row.universe,
            weighted_return: round2(row.weighted),
        })
        .collect();

    // 由新到舊走訪，只在嚴格更高／更低時更新，因此同分保留最近一次。
    let mut high: Option<&RelativeStrengthPoint> = None;
    let mut low: Option<&RelativeStrengthPoint> = None;
    for item in &items {
        if high.is_none_or(|best| item.rs_score > best.rs_score) {
            high = Some(item);
        }
        if low.is_none_or(|worst| item.rs_score < worst.rs_score) {
            low = Some(item);
        }
    }
    let score_change = items
        .first()
        .zip(items.last())
        .map(|(latest, earliest)| i16::from(latest.rs_score) - i16::from(earliest.rs_score));

    RelativeStrengthHistoryResponse {
        stock_symbol: symbol,
        data_as_of: items.first().map(|item| item.date.clone()),
        days,
        latest_score: items.first().map(|item| item.rs_score),
        high_score: high.map(|item| item.rs_score),
        high_date: high.map(|item| item.date.clone()),
        low_score: low.map(|item| item.rs_score),
        low_date: low.map(|item| item.date.clone()),
        score_change,
        crossings: crossings(&items),
        items,
    }
}

/// 相鄰兩個交易日（`items` 由新到舊）穿越 50／80 門檻的轉折，由新到舊；同一天同時穿越
/// 兩個門檻時兩筆都列，高門檻在前。
fn crossings(items: &[RelativeStrengthPoint]) -> Vec<RelativeStrengthCrossing> {
    items
        .windows(2)
        .flat_map(|pair| {
            let (current, previous) = (&pair[0], &pair[1]);
            relative_strength::crossings(previous.rs_score, current.rs_score)
                .into_iter()
                .map(|(threshold, direction)| RelativeStrengthCrossing {
                    date: current.date.clone(),
                    threshold,
                    direction: direction.as_str().to_owned(),
                    from_score: previous.rs_score,
                    to_score: current.rs_score,
                })
        })
        .collect()
}

#[cfg(test)]
mod tests;
