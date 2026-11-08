//! `/market/relative-strength` handler：相對強度排行（RS）。
//!
//! 近 N 日報酬直接取 `daily_forward_return` 基準日 T−N 的 N 日欄位（見
//! [`crate::domain::quote::forward_return`]），不另外存表。加權報酬＝0.4 × 近 60 日
//! ＋ 0.3 × 近 120 日 ＋ 0.3 × 近 240 日（較重近一季），分數是加權報酬在上市櫃普通股（不含 ETF、特別股）中的
//! 百分位（1–99）。市場、產業與代號只篩選輸出，分數與名次一律以全體計算。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Local, NaiveDate};

use super::{
    ensure_stock_exists, error_response, industry_error, market_id_for_stocks, parse_optional_date,
    repository_error, round2,
};
pub(super) use crate::domain::quote::relative_strength::ScoredStock;
#[cfg(test)]
pub(super) use crate::domain::quote::relative_strength::UniverseRow;
use crate::infra::database::repository::quote::PgQuoteRepository;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, RelativeStrengthItem, RelativeStrengthParams, RelativeStrengthResponse,
};

/// 查詢上市櫃股票的相對強度分數與名次。
///
/// 分數以加權報酬（0.4 × 近 60 日 ＋ 0.3 × 近 120 日 ＋ 0.3 × 近 240 日，還原權值）在上市櫃
/// 全體中的百分位計算，99 為最強；上市未滿 240 個交易日的股票不計分。指定的交易日還沒算好
/// 報酬時（例如收盤匯總尚未跑完），改用前一個交易日。
///
/// # Errors
///
/// 參數格式錯誤、`order` 不是 asc／desc、`limit` 超出 1–200 或產業名稱不存在回 422；
/// `symbol` 不存在或查無資料回 404；驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/relative-strength", tag = "data-api", params(RelativeStrengthParams), responses((status = 200, body = RelativeStrengthResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn relative_strength(Query(params): Query<RelativeStrengthParams>) -> Response {
    let date = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let market = params.market.as_deref().unwrap_or("all");
    let Some(market_id) = market_id_for_stocks(market) else {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "market 必須為 all、twse 或 tpex",
        );
    };
    if !matches!(params.order.as_deref(), None | Some("asc" | "desc")) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "order 必須為 asc 或 desc");
    }
    if !(1..=200).contains(&params.limit.unwrap_or(50)) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 200");
    }
    if let Some(response) = industry_error(params.industry.as_deref()).await {
        return response;
    }
    if let Some(symbol) = params.symbol.as_deref()
        && let Some(response) = ensure_stock_exists(symbol).await
    {
        return response;
    }

    let until = date.unwrap_or_else(|| Local::now().date_naive());
    match load_scores(until).await {
        Ok(Some((data_date, scored))) => {
            Json(build_response(data_date, &params, market_id, scored)).into_response()
        }
        Ok(None) => error_response(StatusCode::NOT_FOUND, "查無相對強度資料"),
        Err(response) => *response,
    }
}

/// 載入 `until`（含）之前最近一個已留存的每日 RS 母體；與回測共用分數及名次。
///
/// 最新一個交易日尚未完成 RS 計算時，使用之前最近的已留存交易日；沒有完整
/// 資料回 `Ok(None)`。查詢失敗時回已組好的安全 500 回應，呼叫端直接回傳即可。
pub(super) async fn load_scores(
    until: NaiveDate,
) -> Result<Option<(NaiveDate, Vec<ScoredStock>)>, Box<Response>> {
    PgQuoteRepository::new()
        .fetch_stored_scored_universe(until)
        .await
        .map_err(|error| Box::new(repository_error(error)))
}

/// 依市場、產業、代號篩選已計分的全體，依 `order` 排序後取前 `limit` 筆。
fn build_response(
    data_date: NaiveDate,
    params: &RelativeStrengthParams,
    market_id: i32,
    scored: Vec<ScoredStock>,
) -> RelativeStrengthResponse {
    let universe = scored.len();
    let mut items: Vec<RelativeStrengthItem> = scored
        .into_iter()
        .filter(|stock| {
            let row = &stock.row;
            (market_id == 0 || row.market_id == market_id)
                && params
                    .industry
                    .as_deref()
                    .is_none_or(|industry| row.industry == industry)
                && params
                    .symbol
                    .as_deref()
                    .is_none_or(|symbol| row.stock_symbol == symbol)
        })
        .map(|stock| RelativeStrengthItem {
            rank: stock.rank,
            market: market_label(stock.row.market_id).to_owned(),
            rs_score: stock.rs_score,
            weighted_return: round2(stock.weighted),
            return_20d: stock.row.return_20d.map(round2),
            return_60d: round2(stock.row.return_60d),
            return_120d: round2(stock.row.return_120d),
            return_240d: round2(stock.row.return_240d),
            closing_price: stock.row.closing_price,
            stock_symbol: stock.row.stock_symbol,
            name: stock.row.name,
            industry: stock.row.industry,
        })
        .collect();

    let order = params.order.as_deref().unwrap_or("desc");
    if order == "asc" {
        items.reverse();
    }
    let matched = items.len();
    items.truncate(usize::from(params.limit.unwrap_or(50)));
    RelativeStrengthResponse {
        data_as_of: data_date.to_string(),
        market: params.market.clone().unwrap_or_else(|| "all".to_owned()),
        order: order.to_owned(),
        universe,
        matched,
        items,
    }
}

/// 股票表市場 id 對應的 API 市場代碼。
pub(super) fn market_label(market_id: i32) -> &'static str {
    if market_id == 4 { "tpex" } else { "twse" }
}

#[cfg(test)]
mod tests;
