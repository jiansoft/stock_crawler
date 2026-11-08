//! `/market/institutional-flow` handler：三大法人買賣超排行。
//!
//! 資料來自 `chip_daily`（每晚 21:50 入庫的全市場三大法人買賣超股數）。期間累計淨股數，
//! 估計金額＝每日淨股數 × 當日收盤價加總，依金額排名；另附占期間成交量比例與截至資料日的
//! 連續買賣超天數（入庫時預先算好，不限回看長度）。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Local, NaiveDate};

use super::relative_strength::market_label;
use super::{
    error_response, industry_error, market_id_for_stocks, parse_optional_date, repository_error,
    round2,
};
use crate::domain::chip::flow::{self, ChipFlowQuery};
use crate::infra::database::repository::chip::PgChipRepository;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, InstitutionalFlowItem, InstitutionalFlowParams, InstitutionalFlowResponse,
};

pub(super) use crate::domain::chip::flow::{
    FlowPeriod, INVESTORS, MAX_FLOW_DAYS, StockFlow as FlowRow, StockStreak as StreakRow,
    flow_period,
};

/// `until`（含）之前最近 [`MAX_FLOW_DAYS`] 個有籌碼資料的交易日，由新到舊；查詢失敗時回已組好的
/// 安全 500 回應。
pub(super) async fn chip_dates(until: NaiveDate) -> Result<Vec<NaiveDate>, Box<Response>> {
    PgChipRepository::new()
        .fetch_chip_dates(until)
        .await
        .map_err(|error| Box::new(repository_error(error)))
}

/// 載入 `until`（含）之前最近 `days` 個籌碼交易日、法人別 `investor` 的全體上市櫃淨買賣。
///
/// 沒有任何籌碼資料回 `Ok(None)`；查詢失敗時回已組好的安全 500 回應，呼叫端直接回傳即可。
pub(super) async fn load_flows(
    until: NaiveDate,
    days: usize,
    investor: &str,
) -> Result<Option<(FlowPeriod, Vec<FlowRow>)>, Box<Response>> {
    flow::load_flows(&PgChipRepository::new(), until, days, investor)
        .await
        .map_err(|error| Box::new(repository_error(error)))
}

/// 載入 `until`（含）之前最新籌碼資料日的外資、投信連續買賣超天數。
///
/// 沒有任何籌碼資料回 `Ok(None)`；查詢失敗時回已組好的安全 500 回應。
pub(super) async fn load_streaks(
    until: NaiveDate,
) -> Result<Option<(NaiveDate, Vec<StreakRow>)>, Box<Response>> {
    flow::load_streaks(&PgChipRepository::new(), until)
        .await
        .map_err(|error| Box::new(repository_error(error)))
}

/// 查詢三大法人買賣超排行。
///
/// 累計最近 `days` 個有籌碼資料的交易日，依估計淨買賣金額（每日淨股數 × 當日收盤價）排名；
/// 籌碼資料每晚 21:50 入庫，當天晚上之前的最新資料日是前一個交易日。
///
/// # Errors
///
/// 參數格式錯誤、`days` 超出 1–60、`limit` 超出 1–100、`investor`／`side`／`market` 不合法或
/// 產業名稱不存在回 422；查無籌碼資料回 404；驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/institutional-flow", tag = "data-api", params(InstitutionalFlowParams), responses((status = 200, body = InstitutionalFlowResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn institutional_flow(Query(params): Query<InstitutionalFlowParams>) -> Response {
    let date = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let days = params.days.unwrap_or(1);
    if !(1..=MAX_FLOW_DAYS).contains(&usize::from(days)) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "days 必須介於 1 至 60");
    }
    let investor = params.investor.as_deref().unwrap_or("foreign");
    if !INVESTORS.contains(&investor) {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "investor 必須為 foreign、trust、dealer 或 total",
        );
    }
    let side = params.side.as_deref().unwrap_or("buy");
    if !matches!(side, "buy" | "sell") {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "side 必須為 buy 或 sell");
    }
    let market = params.market.as_deref().unwrap_or("all");
    let Some(market_id) = market_id_for_stocks(market) else {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "market 必須為 all、twse 或 tpex",
        );
    };
    let limit = params.limit.unwrap_or(30);
    if !(1..=100).contains(&limit) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 100");
    }
    if let Some(response) = industry_error(params.industry.as_deref()).await {
        return response;
    }

    let until = date.unwrap_or_else(|| Local::now().date_naive());
    match load_flows(until, usize::from(days), investor).await {
        Ok(Some((period, rows))) => Json(build_response(
            period,
            FlowFilter {
                investor,
                side,
                market,
                market_id,
                industry: params.industry.as_deref(),
                limit: usize::from(limit),
            },
            rows,
        ))
        .into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "查無籌碼資料"),
        Err(response) => *response,
    }
}

/// 排行的篩選與排序條件。
struct FlowFilter<'a> {
    investor: &'a str,
    side: &'a str,
    market: &'a str,
    market_id: i32,
    industry: Option<&'a str>,
    limit: usize,
}

/// 依市場、產業與方向篩選，買超依金額由大到小、賣超由小（負最多）到大，同金額依代號排列。
fn build_response(
    period: FlowPeriod,
    filter: FlowFilter<'_>,
    rows: Vec<FlowRow>,
) -> InstitutionalFlowResponse {
    let buy = filter.side == "buy";
    let mut rows: Vec<FlowRow> = rows
        .into_iter()
        .filter(|row| {
            (filter.market_id == 0 || row.market_id == filter.market_id)
                && filter
                    .industry
                    .is_none_or(|industry| row.industry == industry)
                && if buy {
                    row.net_amount > 0.0
                } else {
                    row.net_amount < 0.0
                }
        })
        .collect();
    rows.sort_by(|a, b| {
        let by_amount = if buy {
            b.net_amount.total_cmp(&a.net_amount)
        } else {
            a.net_amount.total_cmp(&b.net_amount)
        };
        by_amount.then_with(|| a.stock_symbol.cmp(&b.stock_symbol))
    });
    let matched = rows.len();
    let items = rows
        .into_iter()
        .take(filter.limit)
        .enumerate()
        .map(|(index, row)| flow_item(index + 1, row))
        .collect();

    InstitutionalFlowResponse {
        data_as_of: period.data_date.to_string(),
        from: period.from.to_string(),
        days: period.days,
        investor: filter.investor.to_owned(),
        side: filter.side.to_owned(),
        market: filter.market.to_owned(),
        industry: filter.industry.map(str::to_owned),
        matched,
        items,
    }
}

/// 一列轉成排行項目：張數四捨五入、金額取整數元、成交量占比取兩位小數。
fn flow_item(rank: usize, row: FlowRow) -> InstitutionalFlowItem {
    InstitutionalFlowItem {
        rank,
        market: market_label(row.market_id).to_owned(),
        net_lots: (row.net_shares as f64 / 1000.0).round() as i64,
        net_amount: row.net_amount.round(),
        volume_ratio: row
            .volume
            .filter(|volume| *volume > 0.0)
            .map(|volume| round2(row.net_shares as f64 / volume * 100.0)),
        net_shares: row.net_shares,
        buy_days: row.buy_days,
        sell_days: row.sell_days,
        streak: row.streak,
        closing_price: row.closing_price,
        stock_symbol: row.stock_symbol,
        name: row.name,
        industry: row.industry,
    }
}

#[cfg(test)]
mod tests;
