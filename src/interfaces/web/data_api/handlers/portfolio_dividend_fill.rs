//! `/portfolio/dividend-fill` handler：目前持股（含 ETF）近期每次除權息的填息狀態。
//!
//! 填息規則與 `/stocks/{symbol}/dividend-fill` 相同（[`fill::fill_outcome`]），資料來源共用
//! [`PgDividendFillRepository`]。屬個人持股資料。

use std::cmp::Reverse;

use anyhow::Result;
use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Days, Local, NaiveDate};

use super::dividend_fill::into_dto;
use super::{database_error, error_response, repository_error};
use crate::domain::dividend::fill::{self, DividendFillQuery, FillOutcome};
use crate::infra::database;
use crate::infra::database::repository::dividend_fill::PgDividendFillRepository;
use crate::interfaces::web::data_api::dto::{
    DividendFillSummary, ErrorBody, PortfolioDividendFillItem, PortfolioDividendFillParams,
    PortfolioDividendFillResponse,
};

/// 目前持股（未售出、依代號合併）的代號與名稱。
const HOLDINGS_SQL: &str = r#"
SELECT o.security_code AS stock_symbol, COALESCE(s."Name", '') AS name
FROM stock_ownership_details o
LEFT JOIN stocks s ON s.stock_symbol = o.security_code
WHERE NOT o.is_sold
GROUP BY o.security_code, s."Name"
HAVING SUM(o.share_quantity) > 0
ORDER BY o.security_code
"#;

/// 查詢持股近期每次除權息的填息狀態。
///
/// # Errors
///
/// `days` 不在 30–730 回 422、驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/portfolio/dividend-fill", tag = "data-api", params(PortfolioDividendFillParams), responses((status = 200, body = PortfolioDividendFillResponse), (status = 401, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn portfolio_dividend_fill(
    Query(params): Query<PortfolioDividendFillParams>,
) -> Response {
    let days = params.days.unwrap_or(365);
    if !(30..=730).contains(&days) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "days 必須介於 30 至 730");
    }
    let holdings: Vec<(String, String)> = match sqlx::query_as(HOLDINGS_SQL)
        .fetch_all(database::get_connection())
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    let today = Local::now().date_naive();
    match collect(&PgDividendFillRepository::new(), &holdings, today, days).await {
        Ok(outcomes) => Json(response(days, outcomes)).into_response(),
        Err(error) => repository_error(error),
    }
}

/// 每檔持股除權息日在 `today` 往前 `days` 天內的填息結果，附代號與名稱。
async fn collect(
    repository: &impl DividendFillQuery,
    holdings: &[(String, String)],
    today: NaiveDate,
    days: u16,
) -> Result<Vec<(String, String, FillOutcome)>> {
    let since = today - Days::new(u64::from(days));
    let mut collected = Vec::new();
    for (symbol, name) in holdings {
        let outcomes = fill::recent_outcomes(repository, symbol, since, today).await?;
        collected.extend(
            outcomes
                .into_iter()
                .map(|outcome| (symbol.clone(), name.clone(), outcome)),
        );
    }
    Ok(collected)
}

/// 組出回應：未填息的依缺口由大到小在前，已填息的依除權息日由新到舊在後。
fn response(
    days: u16,
    mut outcomes: Vec<(String, String, FillOutcome)>,
) -> PortfolioDividendFillResponse {
    outcomes.sort_by(|(a_symbol, _, a), (b_symbol, _, b)| {
        a.fill_date
            .is_some()
            .cmp(&b.fill_date.is_some())
            .then_with(|| b.gap_percent().cmp(&a.gap_percent()))
            .then_with(|| Reverse(a.event.ex_date).cmp(&Reverse(b.event.ex_date)))
            .then_with(|| a_symbol.cmp(b_symbol))
    });
    let all: Vec<FillOutcome> = outcomes.iter().map(|(_, _, outcome)| *outcome).collect();
    let summary = fill::summarize(&all);
    PortfolioDividendFillResponse {
        days,
        summary: DividendFillSummary {
            events: summary.events,
            filled: summary.filled,
            average_trading_days_to_fill: summary.average_trading_days,
            median_trading_days_to_fill: summary.median_trading_days,
        },
        items: outcomes
            .iter()
            .map(|(symbol, name, outcome)| PortfolioDividendFillItem {
                stock_symbol: symbol.clone(),
                name: name.clone(),
                fill: into_dto(outcome, symbol),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests;
