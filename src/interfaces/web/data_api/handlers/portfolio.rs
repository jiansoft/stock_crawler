//! `/portfolio/summary` handler：目前持股的組合總覽。
//!
//! 部位來自 `stock_ownership_details`（未賣出），同代號多筆合併；報價取
//! `last_daily_quotes`，殖利率取 `yield_rank` 最新一筆（31 天內），F 分數取
//! `financial_health`（評分年度落後 1–2 年內），本益比與股價淨值比取 `last_daily_quotes` 的官方值（0 視為
//! 沒有）。彙總規則在
//! [`crate::domain::portfolio::summary`]。另附每檔的相對強度分數（同
//! [`mod@super::relative_strength`]）與外資、投信連續買賣超天數（同 [`mod@super::institutional_flow`]），
//! 以及最新月營收、最近一季三率與基本面警示（規則在 [`crate::domain::portfolio::health`]）：
//! 月營收超過 2 個月沒更新、財報落後超過 2 季視為過期。

use std::collections::HashMap;

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Local, NaiveDate};
use rust_decimal::Decimal;

use super::institutional_flow::{StreakRow, load_streaks};
use super::relative_strength::{ScoredStock, load_scores};
use super::{
    analytical_decimal_to_f64, database_error, error_response, format_month, repository_error,
};
use crate::domain::portfolio::health::{HoldingFundamentals, HoldingFundamentalsQuery};
use crate::domain::portfolio::summary::{self, Position};
use crate::infra::database::{self, repository::portfolio::PgPortfolioRepository};
use crate::interfaces::web::data_api::dto::{
    ErrorBody, PortfolioHolding, PortfolioSummaryParams, PortfolioSummaryResponse,
    margin_trend_code, warning_code,
};

/// 依代號合併的持有部位與各項指標；`$1 = 0` 時為全部會員。
const POSITIONS_SQL: &str = r#"
WITH held AS (
    SELECT security_code AS stock_symbol,
        SUM(share_quantity)::bigint AS shares,
        -SUM(holding_cost) AS cost,
        SUM(COALESCE(cumulate_dividends_total, 0)) AS dividends_received
    FROM stock_ownership_details
    WHERE NOT is_sold AND ($1::bigint = 0 OR member_id = $1)
    GROUP BY security_code
)
SELECT held.stock_symbol, held.shares, held.cost, held.dividends_received,
    COALESCE(s."Name", '') AS name,
    s.stock_industry_id AS industry_id,
    COALESCE(si.name, '') AS industry_name,
    q.date AS quote_date,
    CASE WHEN q.closing_price > 0 THEN q.closing_price END AS closing_price,
    CASE WHEN y.date >= CURRENT_DATE - 30 THEN y.yield END AS dividend_yield_percent,
    CASE WHEN EXTRACT(YEAR FROM CURRENT_DATE)::int - h.f_score_year BETWEEN 1 AND 2
        THEN h.f_score END AS f_score,
    CASE WHEN q.price_earning_ratio > 0 THEN q.price_earning_ratio END AS pe_ratio,
    CASE WHEN q."price-to-book_ratio" > 0 THEN q."price-to-book_ratio" END AS pb_ratio
FROM held
LEFT JOIN stocks s ON s.stock_symbol = held.stock_symbol
LEFT JOIN stock_industry si ON si.stock_industry_id = s.stock_industry_id
LEFT JOIN last_daily_quotes q ON q.stock_symbol = held.stock_symbol
LEFT JOIN LATERAL (
    SELECT date, yield FROM yield_rank
    WHERE security_code = held.stock_symbol
    ORDER BY date DESC
    LIMIT 1
) y ON TRUE
LEFT JOIN financial_health h ON h.stock_symbol = held.stock_symbol
WHERE held.shares > 0
ORDER BY held.stock_symbol
"#;

#[derive(Default, sqlx::FromRow)]
struct PositionRow {
    stock_symbol: String,
    shares: i64,
    cost: Decimal,
    dividends_received: Decimal,
    name: String,
    industry_id: Option<i32>,
    industry_name: String,
    quote_date: Option<NaiveDate>,
    closing_price: Option<Decimal>,
    dividend_yield_percent: Option<Decimal>,
    f_score: Option<i16>,
    pe_ratio: Option<Decimal>,
    pb_ratio: Option<Decimal>,
}

/// 把營收、三率與警示填入持股。
fn apply_fundamentals(fundamentals: HoldingFundamentals, holding: &mut PortfolioHolding) {
    let round = |value: f64| (value * 100.0).round() / 100.0;
    holding.revenue_month = fundamentals.revenue_month.map(format_month);
    holding.revenue_yoy_percent = analytical_decimal_to_f64(
        fundamentals.revenue_yoy,
        &holding.stock_symbol,
        "revenue_yoy",
    );
    holding.revenue_decline_months = fundamentals.signals.revenue_decline_months;
    holding.margin_period = fundamentals
        .margin_period
        .map(|(year, quarter)| format!("{year}-Q{quarter}"));
    let [gross, operating, net] = fundamentals.margins.map(|margin| margin.map(round));
    holding.gross_margin_percent = gross;
    holding.operating_margin_percent = operating;
    holding.net_margin_percent = net;
    holding.margins_trend = fundamentals
        .signals
        .margin_trend()
        .map(|trend| margin_trend_code(trend).to_owned());
    holding.free_cash_flow_ttm = fundamentals.signals.free_cash_flow_ttm;
    holding.warnings = fundamentals
        .signals
        .warnings()
        .into_iter()
        .map(|warning| warning_code(warning).to_owned())
        .collect();
}

/// 查詢目前持股的組合總覽：市值、權重、未實現損益、已領股利、產業分布與加權指標。
///
/// # Errors
///
/// `member_id` 為負數回 422、驗證失敗回 401；資料庫查詢失敗時記錄內部錯誤並回不含
/// SQL 細節的 500。沒有持股的會員回 200 與空清單。
#[utoipa::path(get, path = "/api/v1/portfolio/summary", tag = "data-api", params(PortfolioSummaryParams), responses((status = 200, body = PortfolioSummaryResponse), (status = 401, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn portfolio_summary(Query(params): Query<PortfolioSummaryParams>) -> Response {
    let member_id = params.member_id.unwrap_or(0);
    if member_id < 0 {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "member_id 不可為負數");
    }
    let today = Local::now().date_naive();
    let (rows, scores, streaks) = tokio::join!(
        sqlx::query_as::<_, PositionRow>(POSITIONS_SQL)
            .bind(member_id)
            .fetch_all(database::get_connection()),
        load_scores(today),
        load_streaks(today),
    );
    let rows = match rows {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    let scores = match scores {
        Ok(scores) => scores,
        Err(response) => return *response,
    };
    let streaks = match streaks {
        Ok(streaks) => streaks,
        Err(response) => return *response,
    };
    let symbols: Vec<String> = rows.iter().map(|row| row.stock_symbol.clone()).collect();
    let fundamentals = match PgPortfolioRepository::new()
        .fetch_holding_fundamentals(&symbols)
        .await
    {
        Ok(fundamentals) => fundamentals,
        Err(error) => return repository_error(error),
    };
    let mut response = build_response(member_id, rows, fundamentals);
    attach_indicators(&mut response, scores, streaks);
    Json(response).into_response()
}

/// 依代號把相對強度分數與連續買賣超天數附到各持股，並填入兩者的資料日。
fn attach_indicators(
    response: &mut PortfolioSummaryResponse,
    scores: Option<(NaiveDate, Vec<ScoredStock>)>,
    streaks: Option<(NaiveDate, Vec<StreakRow>)>,
) {
    let mut rs_scores = HashMap::new();
    if let Some((date, scored)) = scores {
        response.rs_as_of = Some(date.to_string());
        rs_scores.extend(
            scored
                .into_iter()
                .map(|stock| (stock.row.stock_symbol, stock.rs_score)),
        );
    }
    let mut chip = HashMap::new();
    if let Some((date, rows)) = streaks {
        response.chip_as_of = Some(date.to_string());
        chip.extend(
            rows.into_iter()
                .map(|row| (row.stock_symbol, (row.foreign_streak, row.trust_streak))),
        );
    }
    for holding in &mut response.holdings {
        holding.rs_score = rs_scores.get(&holding.stock_symbol).copied();
        if let Some(&(foreign, trust)) = chip.get(&holding.stock_symbol) {
            holding.foreign_streak = Some(foreign);
            holding.trust_streak = Some(trust);
        }
    }
}

/// 由查詢結果組出回應並依代號附上營收、三率與警示；`data_as_of` 取各持股報價日中最新的一天。
fn build_response(
    member_id: i64,
    rows: Vec<PositionRow>,
    fundamentals: Vec<HoldingFundamentals>,
) -> PortfolioSummaryResponse {
    let data_as_of = rows.iter().filter_map(|row| row.quote_date).max();
    let mut fundamentals: HashMap<String, HoldingFundamentals> = fundamentals
        .into_iter()
        .map(|fundamentals| (fundamentals.stock_symbol.clone(), fundamentals))
        .collect();
    let positions = rows
        .into_iter()
        .map(|row| Position {
            stock_symbol: row.stock_symbol,
            name: row.name,
            industry_id: row.industry_id,
            industry_name: row.industry_name,
            shares: row.shares,
            cost: row.cost,
            dividends_received: row.dividends_received,
            closing_price: row.closing_price,
            dividend_yield_percent: row.dividend_yield_percent,
            f_score: row.f_score,
            pe_ratio: row.pe_ratio,
            pb_ratio: row.pb_ratio,
        })
        .collect();
    let mut response = PortfolioSummaryResponse::new(
        member_id,
        data_as_of.map(|date| date.to_string()),
        summary::summarize(positions),
    );
    for holding in &mut response.holdings {
        if let Some(fundamentals) = fundamentals.remove(&holding.stock_symbol) {
            apply_fundamentals(fundamentals, holding);
        }
    }
    response.warning_symbols = response
        .holdings
        .iter()
        .filter(|holding| !holding.warnings.is_empty())
        .count();
    response
}

#[cfg(test)]
mod tests;
