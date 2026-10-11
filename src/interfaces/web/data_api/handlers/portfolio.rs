//! `/portfolio/summary` handler：目前持股的組合總覽。
//!
//! 部位來自 `stock_ownership_details`（未賣出），同代號多筆合併；報價取
//! `last_daily_quotes`，殖利率取 `yield_rank` 最新一筆（31 天內），F 分數取
//! `financial_health`（評分年度落後 1–2 年內）。彙總規則在
//! [`crate::domain::portfolio::summary`]。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::{database_error, error_response};
use crate::domain::portfolio::summary::{self, Position};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, PortfolioSummaryParams, PortfolioSummaryResponse,
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
        THEN h.f_score END AS f_score
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

#[derive(sqlx::FromRow)]
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
    let rows: Vec<PositionRow> = match sqlx::query_as(POSITIONS_SQL)
        .bind(member_id)
        .fetch_all(database::get_connection())
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    Json(build_response(member_id, rows)).into_response()
}

/// 由查詢結果組出回應；`data_as_of` 取各持股報價日中最新的一天。
fn build_response(member_id: i64, rows: Vec<PositionRow>) -> PortfolioSummaryResponse {
    let data_as_of = rows.iter().filter_map(|row| row.quote_date).max();
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
        })
        .collect();
    PortfolioSummaryResponse::new(
        member_id,
        data_as_of.map(|date| date.to_string()),
        summary::summarize(positions),
    )
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    /// 報價日取最新、各欄位原樣轉入領域彙總。
    #[test]
    fn build_response_uses_latest_quote_date() {
        let row = |symbol: &str, day: u32, price: Option<Decimal>| PositionRow {
            stock_symbol: symbol.to_owned(),
            shares: 1_000,
            cost: dec!(10_000),
            dividends_received: dec!(120),
            name: "測試".to_owned(),
            industry_id: Some(1),
            industry_name: "水泥工業".to_owned(),
            quote_date: NaiveDate::from_ymd_opt(2026, 4, day),
            closing_price: price,
            dividend_yield_percent: Some(dec!(3.5)),
            f_score: Some(7),
        };
        let response = build_response(
            2,
            vec![row("1101", 29, Some(dec!(12))), row("1102", 30, None)],
        );
        assert_eq!(response.member_id, 2);
        assert_eq!(response.data_as_of.as_deref(), Some("2026-04-30"));
        assert_eq!((response.symbols, response.market_value), (2, 12_000.0));
        assert_eq!(response.dividends_received, 240.0);
        assert_eq!(response.holdings[0].stock_symbol, "1101");
        assert_eq!(response.holdings[0].f_score, Some(7));
        assert_eq!(response.holdings[1].closing_price, None);
        assert_eq!(response.industries[0].symbols, 2);

        let empty = build_response(0, Vec::new());
        assert!(empty.data_as_of.is_none() && empty.holdings.is_empty());
    }
}
