//! `/stocks/{symbol}/financial-health` handler：以年度三大報表計算 Piotroski F 分數。
//!
//! 評分規則在 [`crate::domain::financial::health`]，與每日全市場重算共用。
//! 損益與現金流量用年度報表，資產負債表取各年第 4 季季末；
//! 同一期 Yahoo 與 BigGo 並存時取 Yahoo。

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use super::{database_error, ensure_stock_exists, error_response};
use crate::domain::financial::health::{self, YearFigures};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, FinancialHealthParams, FinancialHealthResponse, HealthCriterion,
};

/// 以 Piotroski F 分數評估單一股票的年度財務體質。
///
/// 回傳 9 項指標各自的本期數值、比較基準與是否得分；資料不足或不適用的指標
/// 為 `null`、不計分，`evaluated` 是可判定的指標數。
///
/// # Errors
///
/// `fiscal_year` 不在 2000–2100 回 422、股票不存在回 404、驗證失敗回 401；
/// 資料庫查詢失敗時記錄內部錯誤並回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/financial-health", tag = "data-api", params(("symbol" = String, Path, description = "股票代號"), FinancialHealthParams), responses((status = 200, body = FinancialHealthResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn financial_health(
    Path(symbol): Path<String>,
    Query(params): Query<FinancialHealthParams>,
) -> Response {
    if params
        .fiscal_year
        .is_some_and(|year| !(2000..=2100).contains(&year))
    {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "fiscal_year 必須介於 2000 至 2100",
        );
    }
    if let Some(response) = ensure_stock_exists(&symbol).await {
        return response;
    }
    let pool = database::get_connection();

    let fiscal_year = match params.fiscal_year {
        Some(year) => Some(year),
        None => match sqlx::query_scalar::<_, Option<i32>>(
            "SELECT max(fiscal_year) FROM income_statement WHERE stock_symbol = $1 AND period_type = 'annual'",
        )
        .bind(&symbol)
        .fetch_one(pool)
        .await
        {
            Ok(year) => year,
            Err(error) => return database_error(error),
        },
    };

    let (current, previous) = match fiscal_year {
        Some(year) => {
            // 兩個年度各取一列：三張表都用 DISTINCT ON 讓同一年只留一列、Yahoo 優先；
            // 沒有資料的年度由 VALUES 補出全 NULL 的列。
            let rows: Vec<(i32, YearFigures)> = match sqlx::query_as::<_, YearRow>(
                r#"WITH years(fiscal_year) AS (VALUES ($2::int), ($2::int - 1)),
inc AS (SELECT DISTINCT ON (fiscal_year) fiscal_year, revenue, gross_profit, net_income FROM income_statement WHERE stock_symbol = $1 AND period_type = 'annual' AND fiscal_year IN ($2::int, $2::int - 1) ORDER BY fiscal_year, (source = 'yahoo') DESC, source),
cf AS (SELECT DISTINCT ON (fiscal_year) fiscal_year, operating_cash_flow FROM cash_flow_statement WHERE stock_symbol = $1 AND period_type = 'annual' AND fiscal_year IN ($2::int, $2::int - 1) ORDER BY fiscal_year, (source = 'yahoo') DESC, source),
bs AS (SELECT DISTINCT ON (fiscal_year) fiscal_year, total_assets, current_assets, current_liabilities, non_current_liabilities, share_capital FROM balance_sheet WHERE stock_symbol = $1 AND period_type = 'single' AND quarter = 'Q4' AND fiscal_year IN ($2::int, $2::int - 1) ORDER BY fiscal_year, (source = 'yahoo') DESC, source)
SELECT years.fiscal_year, inc.revenue, inc.gross_profit, inc.net_income, cf.operating_cash_flow, bs.total_assets, bs.current_assets, bs.current_liabilities, bs.non_current_liabilities, bs.share_capital
FROM years LEFT JOIN inc USING (fiscal_year) LEFT JOIN cf USING (fiscal_year) LEFT JOIN bs USING (fiscal_year)"#,
            )
            .bind(&symbol)
            .bind(year)
            .fetch_all(pool)
            .await
            {
                Ok(rows) => rows
                    .into_iter()
                    .map(|row| (row.fiscal_year, row.figures()))
                    .collect(),
                Err(error) => return database_error(error),
            };
            let figures = |target: i32| {
                rows.iter()
                    .find(|(row_year, _)| *row_year == target)
                    .map(|(_, figures)| *figures)
                    .unwrap_or_default()
            };
            (figures(year), figures(year - 1))
        }
        None => (YearFigures::default(), YearFigures::default()),
    };

    let criteria = health::piotroski(&current, &previous);
    let (score, evaluated) = health::tally(&criteria);
    Json(FinancialHealthResponse {
        stock_symbol: symbol,
        fiscal_year,
        compared_with: fiscal_year.map(|year| year - 1),
        score,
        evaluated,
        max_score: health::MAX_SCORE,
        criteria: criteria.into_iter().map(into_dto).collect(),
    })
    .into_response()
}

fn into_dto(criterion: health::HealthCriterion) -> HealthCriterion {
    HealthCriterion {
        key: criterion.key,
        name: criterion.name,
        category: criterion.category,
        passed: criterion.passed,
        value: criterion.value,
        baseline: criterion.baseline,
        unit: criterion.unit,
    }
}

/// SQL 回傳的一列：年度加上該年度的數值（新台幣千元）。
#[derive(sqlx::FromRow)]
struct YearRow {
    fiscal_year: i32,
    revenue: Option<i64>,
    gross_profit: Option<i64>,
    net_income: Option<i64>,
    operating_cash_flow: Option<i64>,
    total_assets: Option<i64>,
    current_assets: Option<i64>,
    current_liabilities: Option<i64>,
    non_current_liabilities: Option<i64>,
    share_capital: Option<i64>,
}

impl YearRow {
    fn figures(&self) -> YearFigures {
        YearFigures {
            revenue: self.revenue,
            gross_profit: self.gross_profit,
            net_income: self.net_income,
            operating_cash_flow: self.operating_cash_flow,
            total_assets: self.total_assets,
            current_assets: self.current_assets,
            current_liabilities: self.current_liabilities,
            non_current_liabilities: self.non_current_liabilities,
            share_capital: self.share_capital,
        }
    }
}
