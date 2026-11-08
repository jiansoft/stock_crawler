//! `/market/industry-strength` handler：各產業成分股相對強度的分布，用來看資金輪動到哪些產業。
//!
//! 個股分數沿用 [`mod@super::relative_strength`] 的上市櫃普通股（不含 ETF、特別股）計分；產業強度＝成分股 RS 的中位數，
//! 另附強勢（RS ≥ 80）與弱勢（RS ≤ 20）占比，並與約 20 個交易日前的中位數比較。

use std::collections::BTreeMap;

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Local, NaiveDate};

use super::relative_strength::{ScoredStock, load_scores};
use super::{
    error_response, market_id_for_stocks, parse_optional_date, repository_error, round2, round4,
};
use crate::domain::quote::forward_return::ForwardReturnRepository;
use crate::domain::quote::relative_strength::{group_by_industry, median_score};
use crate::infra::database::repository::quote::PgQuoteRepository;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, IndustryLeader, IndustryStrengthItem, IndustryStrengthParams,
    IndustryStrengthResponse,
};

/// 比較基準：資料日往前第幾個交易日。
const PRIOR_LAG: usize = 20;

/// 強勢門檻（含）。
const STRONG: u8 = 80;

/// 弱勢門檻（含）。
const WEAK: u8 = 20;

/// 每個產業列出的領先股檔數。
const LEADERS: usize = 3;

/// 查詢各產業的相對強度：成分股 RS 中位數、強弱勢占比、平均加權報酬與約 20 個交易日前的比較。
///
/// 個股分數以上市櫃普通股（不含 ETF、特別股）計算（與 `/market/relative-strength` 相同），`market` 只決定各產業納入
/// 哪些股票；未分類的股票不列入任何產業。
///
/// # Errors
///
/// 參數格式錯誤、`order` 不是 asc／desc 或 `min_stocks` 超出 1–50 回 422；查無相對強度資料回 404；
/// 驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/industry-strength", tag = "data-api", params(IndustryStrengthParams), responses((status = 200, body = IndustryStrengthResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn industry_strength(Query(params): Query<IndustryStrengthParams>) -> Response {
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
    let order = params.order.as_deref().unwrap_or("desc");
    if !matches!(order, "asc" | "desc") {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "order 必須為 asc 或 desc");
    }
    let min_stocks = params.min_stocks.unwrap_or(3);
    if !(1..=50).contains(&min_stocks) {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "min_stocks 必須介於 1 至 50",
        );
    }

    let until = date.unwrap_or_else(|| Local::now().date_naive());
    let (data_date, current) = match load_scores(until).await {
        Ok(Some(scores)) => scores,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "查無相對強度資料"),
        Err(response) => return *response,
    };
    let prior_until = match PgQuoteRepository::new()
        .fetch_trading_dates(data_date, PRIOR_LAG + 1)
        .await
    {
        Ok(dates) => dates.get(PRIOR_LAG).copied(),
        Err(error) => return repository_error(error),
    };
    let prior = match prior_until {
        Some(prior_until) => match load_scores(prior_until).await {
            Ok(scores) => scores,
            Err(response) => return *response,
        },
        None => None,
    };

    Json(build_response(
        data_date,
        market,
        market_id,
        usize::from(min_stocks),
        order,
        current,
        prior,
    ))
    .into_response()
}

/// 組成回應：各產業統計、與前期中位數比較，依中位數排名（同中位數同名次、再依產業名稱）。
fn build_response(
    data_date: NaiveDate,
    market: &str,
    market_id: i32,
    min_stocks: usize,
    order: &str,
    current: Vec<ScoredStock>,
    prior: Option<(NaiveDate, Vec<ScoredStock>)>,
) -> IndustryStrengthResponse {
    let universe = current.len();
    let (prior_as_of, prior_medians) = match prior {
        Some((date, scored)) => (
            Some(date.to_string()),
            group_by_industry(scored, market_id)
                .into_iter()
                .filter_map(|(industry, stocks)| Some((industry, median_score(&stocks)?)))
                .collect(),
        ),
        None => (None, BTreeMap::new()),
    };

    let mut items: Vec<IndustryStrengthItem> = group_by_industry(current, market_id)
        .into_iter()
        .filter(|(_, stocks)| stocks.len() >= min_stocks)
        .filter_map(|(industry, stocks)| {
            let median = median_score(&stocks)?;
            let count = stocks.len() as f64;
            let ratio = |keep: fn(u8) -> bool| {
                round4(stocks.iter().filter(|stock| keep(stock.rs_score)).count() as f64 / count)
            };
            let prior_median = prior_medians.get(&industry).copied();
            Some(IndustryStrengthItem {
                rank: 0,
                stocks: stocks.len(),
                median_rs_score: median,
                strong_ratio: ratio(|score| score >= STRONG),
                weak_ratio: ratio(|score| score <= WEAK),
                avg_weighted_return: round2(
                    stocks.iter().map(|stock| stock.weighted).sum::<f64>() / count,
                ),
                prior_median_rs_score: prior_median,
                median_change: prior_median.map(|prior| median - prior),
                leaders: stocks
                    .iter()
                    .take(LEADERS)
                    .map(|stock| IndustryLeader {
                        stock_symbol: stock.row.stock_symbol.clone(),
                        name: stock.row.name.clone(),
                        rs_score: stock.rs_score,
                    })
                    .collect(),
                industry,
            })
        })
        .collect();

    items.sort_by(|a, b| {
        b.median_rs_score
            .total_cmp(&a.median_rs_score)
            .then_with(|| a.industry.cmp(&b.industry))
    });
    for index in 0..items.len() {
        items[index].rank =
            if index > 0 && items[index].median_rs_score == items[index - 1].median_rs_score {
                items[index - 1].rank
            } else {
                index + 1
            };
    }
    if order == "asc" {
        items.reverse();
    }

    IndustryStrengthResponse {
        data_as_of: data_date.to_string(),
        prior_as_of,
        market: market.to_owned(),
        order: order.to_owned(),
        universe,
        items,
    }
}

#[cfg(test)]
mod tests;
