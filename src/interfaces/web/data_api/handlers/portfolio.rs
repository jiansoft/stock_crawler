//! `/portfolio/summary` handler：目前持股的組合總覽。
//!
//! 部位來自 `stock_ownership_details`（未賣出），同代號多筆合併；報價取
//! `last_daily_quotes`，殖利率取 `yield_rank` 最新一筆（31 天內），F 分數取
//! `financial_health`（評分年度落後 1–2 年內）。彙總規則在
//! [`crate::domain::portfolio::summary`]。另附每檔的相對強度分數（同
//! [`super::relative_strength`]）與外資、投信連續買賣超天數（同 [`super::institutional_flow`]），
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
use super::{analytical_decimal_to_f64, database_error, error_response, format_month};
use crate::core::declare::Industry;
use crate::domain::portfolio::health::FundamentalSignals;
use crate::domain::portfolio::summary::{self, Position};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, PortfolioHolding, PortfolioSummaryParams, PortfolioSummaryResponse,
    margin_trend_code, warning_code,
};

/// 依代號合併的持有部位與各項指標；`$1 = 0` 時為全部會員，`$2` 為不取三率的產業（金融保險業）。
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
    rv.revenue_month, rv.revenue_yoy, rv.revenue_decline_months,
    m.margin_year, m.margin_quarter, m.gross_margin, m.operating_margin, m.net_margin,
    m.last_year_gross_margin, m.last_year_operating_margin, m.last_year_net_margin
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
LEFT JOIN LATERAL (
    -- 最近 12 個月營收：最新一個月的年增率，與從最新一個月起連續年減（月份不中斷）的月數。
    SELECT
        MAX(revenue_month) FILTER (WHERE rn = 1) AS revenue_month,
        MAX(yoy) FILTER (WHERE rn = 1) AS revenue_yoy,
        COALESCE(
            MIN(rn) FILTER (WHERE NOT (yoy < 0 AND ord = first_ord - rn + 1)) - 1,
            COUNT(*)
        )::smallint AS revenue_decline_months
    FROM (
        SELECT revenue_month, yoy, ord,
            ROW_NUMBER() OVER (ORDER BY ord DESC) AS rn,
            FIRST_VALUE(ord) OVER (ORDER BY ord DESC) AS first_ord
        FROM (
            SELECT "Date" AS revenue_month,
                "ComparedWithLastYearSameMonth" AS yoy,
                ("Date" / 100)::int * 12 + ("Date" % 100)::int AS ord
            FROM "Revenue"
            WHERE "SecurityCode" = held.stock_symbol
            ORDER BY "Date" DESC
            LIMIT 12
        ) recent
    ) ranked
    HAVING MAX(ord) >= EXTRACT(YEAR FROM CURRENT_DATE)::int * 12
        + EXTRACT(MONTH FROM CURRENT_DATE)::int - 2
) rv ON TRUE
LEFT JOIN LATERAL (
    -- Yahoo 最新一季單季損益表的三率與去年同季的三率（營收為 0 時為 NULL）；金融保險業的營收口徑
    -- 與一般公司不同，三率沒有可比性，不取。
    SELECT cur.fiscal_year AS margin_year,
        SUBSTRING(cur.quarter FROM 2)::smallint AS margin_quarter,
        cur.gross_profit::float8 / NULLIF(cur.revenue, 0) * 100 AS gross_margin,
        cur.operating_profit::float8 / NULLIF(cur.revenue, 0) * 100 AS operating_margin,
        cur.net_income::float8 / NULLIF(cur.revenue, 0) * 100 AS net_margin,
        prev.gross_profit::float8 / NULLIF(prev.revenue, 0) * 100 AS last_year_gross_margin,
        prev.operating_profit::float8 / NULLIF(prev.revenue, 0) * 100 AS last_year_operating_margin,
        prev.net_income::float8 / NULLIF(prev.revenue, 0) * 100 AS last_year_net_margin
    FROM (
        SELECT fiscal_year, quarter, revenue, gross_profit, operating_profit, net_income
        FROM income_statement
        WHERE source = 'yahoo'
          AND stock_symbol = held.stock_symbol
          AND period_type = 'single'
          AND s.stock_industry_id IS DISTINCT FROM $2
        ORDER BY fiscal_year DESC, quarter DESC
        LIMIT 1
    ) cur
    LEFT JOIN income_statement prev
           ON prev.source = 'yahoo'
          AND prev.stock_symbol = held.stock_symbol
          AND prev.period_type = 'single'
          AND prev.fiscal_year = cur.fiscal_year - 1
          AND prev.quarter = cur.quarter
    WHERE (EXTRACT(YEAR FROM CURRENT_DATE)::int * 4 + EXTRACT(QUARTER FROM CURRENT_DATE)::int)
        - (cur.fiscal_year * 4 + SUBSTRING(cur.quarter FROM 2)::int) BETWEEN 0 AND 2
) m ON TRUE
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
    revenue_month: Option<i64>,
    revenue_yoy: Option<Decimal>,
    revenue_decline_months: Option<i16>,
    margin_year: Option<i32>,
    margin_quarter: Option<i16>,
    gross_margin: Option<f64>,
    operating_margin: Option<f64>,
    net_margin: Option<f64>,
    last_year_gross_margin: Option<f64>,
    last_year_operating_margin: Option<f64>,
    last_year_net_margin: Option<f64>,
}

/// 一檔持股的營收與三率欄位，在組出領域彙總前先從查詢列取出。
#[derive(Debug, Clone, Default, PartialEq)]
struct Fundamentals {
    revenue_month: Option<i64>,
    revenue_yoy: Option<Decimal>,
    margin_period: Option<String>,
    margins: [Option<f64>; 3],
    signals: FundamentalSignals,
}

impl Fundamentals {
    fn from_row(row: &PositionRow) -> Self {
        let change = |current: Option<f64>, last_year: Option<f64>| Some(current? - last_year?);
        Self {
            revenue_month: row.revenue_month,
            revenue_yoy: row.revenue_yoy,
            margin_period: row
                .margin_year
                .zip(row.margin_quarter)
                .map(|(year, quarter)| format!("{year}-Q{quarter}")),
            margins: [row.gross_margin, row.operating_margin, row.net_margin],
            signals: FundamentalSignals {
                revenue_decline_months: row
                    .revenue_decline_months
                    .and_then(|months| u8::try_from(months).ok()),
                operating_margin: row.operating_margin,
                margin_changes: [
                    change(row.gross_margin, row.last_year_gross_margin),
                    change(row.operating_margin, row.last_year_operating_margin),
                    change(row.net_margin, row.last_year_net_margin),
                ],
            },
        }
    }

    /// 把營收、三率與警示填入持股。
    fn apply(self, holding: &mut PortfolioHolding) {
        let round = |value: f64| (value * 100.0).round() / 100.0;
        holding.revenue_month = self.revenue_month.map(format_month);
        holding.revenue_yoy_percent =
            analytical_decimal_to_f64(self.revenue_yoy, &holding.stock_symbol, "revenue_yoy");
        holding.revenue_decline_months = self.signals.revenue_decline_months;
        holding.margin_period = self.margin_period;
        let [gross, operating, net] = self.margins.map(|margin| margin.map(round));
        holding.gross_margin_percent = gross;
        holding.operating_margin_percent = operating;
        holding.net_margin_percent = net;
        holding.margins_trend = self
            .signals
            .margin_trend()
            .map(|trend| margin_trend_code(trend).to_owned());
        holding.warnings = self
            .signals
            .warnings()
            .into_iter()
            .map(|warning| warning_code(warning).to_owned())
            .collect();
    }
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
            .bind(Industry::FinanceInsurance as i32)
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
    let mut response = build_response(member_id, rows);
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

/// 由查詢結果組出回應；`data_as_of` 取各持股報價日中最新的一天。
fn build_response(member_id: i64, rows: Vec<PositionRow>) -> PortfolioSummaryResponse {
    let data_as_of = rows.iter().filter_map(|row| row.quote_date).max();
    let mut fundamentals: HashMap<String, Fundamentals> = rows
        .iter()
        .map(|row| (row.stock_symbol.clone(), Fundamentals::from_row(row)))
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
        })
        .collect();
    let mut response = PortfolioSummaryResponse::new(
        member_id,
        data_as_of.map(|date| date.to_string()),
        summary::summarize(positions),
    );
    for holding in &mut response.holdings {
        if let Some(fundamentals) = fundamentals.remove(&holding.stock_symbol) {
            fundamentals.apply(holding);
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
            ..Default::default()
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

    /// 依代號附上分數與連續天數；沒有分數或籌碼的持股維持 null；資料日只在有資料時填入。
    #[test]
    fn attach_indicators_matches_by_symbol() {
        use super::super::relative_strength::UniverseRow;

        let position = |symbol: &str| PositionRow {
            stock_symbol: symbol.to_owned(),
            shares: 1_000,
            cost: dec!(10_000),
            dividends_received: dec!(0),
            name: symbol.to_owned(),
            industry_id: None,
            industry_name: String::new(),
            quote_date: NaiveDate::from_ymd_opt(2026, 10, 14),
            closing_price: Some(dec!(10)),
            dividend_yield_percent: None,
            f_score: None,
            ..Default::default()
        };
        let scored = |symbol: &str, rs_score: u8| ScoredStock {
            rank: 1,
            rs_score,
            weighted: 1.0,
            row: UniverseRow {
                stock_symbol: symbol.to_owned(),
                name: String::new(),
                market_id: 2,
                industry: String::new(),
                return_20d: None,
                return_60d: 0.0,
                return_120d: 0.0,
                return_240d: 0.0,
                closing_price: None,
            },
        };
        let day = |d: u32| NaiveDate::from_ymd_opt(2026, 10, d).expect("合法日期");
        let mut response = build_response(1, vec![position("0050"), position("2330")]);
        attach_indicators(
            &mut response,
            Some((day(14), vec![scored("2330", 79), scored("2317", 50)])),
            Some((
                day(13),
                vec![StreakRow {
                    stock_symbol: "2330".to_owned(),
                    foreign_streak: -6,
                    trust_streak: 11,
                }],
            )),
        );
        assert_eq!(response.rs_as_of.as_deref(), Some("2026-10-14"));
        assert_eq!(response.chip_as_of.as_deref(), Some("2026-10-13"));
        let by_symbol = |symbol: &str| {
            response
                .holdings
                .iter()
                .find(|holding| holding.stock_symbol == symbol)
                .map(|holding| {
                    (
                        holding.rs_score,
                        holding.foreign_streak,
                        holding.trust_streak,
                    )
                })
        };
        assert_eq!(by_symbol("2330"), Some((Some(79), Some(-6), Some(11))));
        assert_eq!(by_symbol("0050"), Some((None, None, None)));

        let mut bare = build_response(1, vec![position("2330")]);
        attach_indicators(&mut bare, None, None);
        assert!(bare.rs_as_of.is_none() && bare.chip_as_of.is_none());
        assert!(bare.holdings[0].rs_score.is_none());
    }

    /// 營收、三率與警示依代號填入持股：三率四捨五入到小數 2 位、三率三降與營收連 3 月年減發警示，
    /// 沒有營收與財報的持股維持 `null` 與空警示；`warning_symbols` 只算有警示的檔數。
    #[test]
    fn build_response_attaches_fundamentals_and_warnings() {
        let warned = PositionRow {
            stock_symbol: "2330".to_owned(),
            shares: 1_000,
            closing_price: Some(dec!(100)),
            revenue_month: Some(202609),
            revenue_yoy: Some(dec!(-12.5)),
            revenue_decline_months: Some(3),
            margin_year: Some(2026),
            margin_quarter: Some(2),
            gross_margin: Some(40.126),
            operating_margin: Some(-1.5),
            net_margin: Some(2.0),
            last_year_gross_margin: Some(45.0),
            last_year_operating_margin: Some(3.0),
            last_year_net_margin: Some(4.0),
            ..Default::default()
        };
        let bare = PositionRow {
            stock_symbol: "0050".to_owned(),
            shares: 1_000,
            closing_price: Some(dec!(50)),
            ..Default::default()
        };
        let response = build_response(0, vec![warned, bare]);
        assert_eq!(response.warning_symbols, 1);
        let holding = |symbol: &str| {
            response
                .holdings
                .iter()
                .find(|holding| holding.stock_symbol == symbol)
                .expect("holding")
        };
        let tsmc = holding("2330");
        assert_eq!(
            (
                tsmc.revenue_month.as_deref(),
                tsmc.revenue_yoy_percent,
                tsmc.revenue_decline_months
            ),
            (Some("2026-09"), Some(-12.5), Some(3))
        );
        assert_eq!(tsmc.margin_period.as_deref(), Some("2026-Q2"));
        assert_eq!(
            (
                tsmc.gross_margin_percent,
                tsmc.operating_margin_percent,
                tsmc.net_margin_percent
            ),
            (Some(40.13), Some(-1.5), Some(2.0))
        );
        assert_eq!(tsmc.margins_trend.as_deref(), Some("all_down"));
        assert_eq!(
            tsmc.warnings,
            ["revenue_decline", "margins_all_down", "operating_loss"]
        );
        let etf = holding("0050");
        assert!(etf.revenue_month.is_none() && etf.margin_period.is_none());
        assert!(etf.margins_trend.is_none() && etf.warnings.is_empty());
    }
}
