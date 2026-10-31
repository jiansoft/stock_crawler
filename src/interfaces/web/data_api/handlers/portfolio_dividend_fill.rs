//! `/portfolio/dividend-fill` handler：目前持股（含 ETF）近期每次除權息的填息狀態。
//!
//! 填息規則與 `/stocks/{symbol}/dividend-fill` 相同（[`fill::fill_outcome`]），資料來源共用
//! [`PgDividendFillRepository`]。每列附同殖利率組的全市場基準（與 `/market/dividend-fill-stats`
//! 共用快取）；基準查詢失敗只記警告、`peer` 為 `null`，不影響持股資料。屬個人持股資料。

use std::cmp::Reverse;

use anyhow::Result;
use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Days, Local, NaiveDate};
use sqlx::PgPool;

use super::dividend_fill::{into_dto, summary_dto};
use super::market_dividend_fill::bucket_dto;
use super::{database_error, error_response, repository_error};
use crate::domain::dividend::fill::{self, DividendFillQuery, FillOutcome, FillSummary};
use crate::domain::dividend::fill_stats::{BucketStats, YieldBucket, bucket_stats};
use crate::infra::cache::dividend_fill_stats;
use crate::infra::database;
use crate::infra::database::repository::dividend_fill::PgDividendFillRepository;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, PortfolioDividendFillItem, PortfolioDividendFillParams,
    PortfolioDividendFillResponse,
};

/// 歷年填息對照回看的天數（約 5 年）。
const HISTORY_DAYS: u64 = 5 * 365 + 1;

/// 同組基準的統計年數；與 [`HISTORY_DAYS`] 一致（`stats_window` 同樣是 5 × 365 + 1 天）。
const PEER_YEARS: u8 = 5;

/// 一檔持股回看期間內的填息結果，附近 5 年的填息統計。
struct HoldingFill {
    symbol: String,
    name: String,
    recent: Vec<FillOutcome>,
    history: FillSummary,
}

/// 目前持股（未售出、依代號合併）的代號與名稱；`$1` 為 0 時是全部會員合計。
const HOLDINGS_SQL: &str = r#"
SELECT o.security_code AS stock_symbol, COALESCE(s."Name", '') AS name
FROM stock_ownership_details o
LEFT JOIN stocks s ON s.stock_symbol = o.security_code
WHERE NOT o.is_sold AND ($1::bigint = 0 OR o.member_id = $1)
GROUP BY o.security_code, s."Name"
HAVING SUM(o.share_quantity) > 0
ORDER BY o.security_code
"#;

/// 查詢持股近期每次除權息的填息狀態。
///
/// # Errors
///
/// `days` 不在 30–730 或 `member_id` 為負數回 422、驗證失敗回 401；資料庫查詢失敗回不含 SQL
/// 細節的 500。
#[utoipa::path(get, path = "/api/v1/portfolio/dividend-fill", tag = "data-api", params(PortfolioDividendFillParams), responses((status = 200, body = PortfolioDividendFillResponse), (status = 401, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn portfolio_dividend_fill(
    Query(params): Query<PortfolioDividendFillParams>,
) -> Response {
    let days = params.days.unwrap_or(365);
    if !(30..=730).contains(&days) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "days 必須介於 30 至 730");
    }
    let member_id = params.member_id.unwrap_or(0);
    if member_id < 0 {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "member_id 不可為負數");
    }
    let holdings = match fetch_holdings(database::get_connection(), member_id).await {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    let today = Local::now().date_naive();
    let repository = PgDividendFillRepository::new();
    let outcomes = match collect(&repository, &holdings, today, days).await {
        Ok(outcomes) => outcomes,
        Err(error) => return repository_error(error),
    };
    let peers =
        match dividend_fill_stats::market_fill_samples_for(&repository, today, PEER_YEARS).await {
            Ok(samples) => bucket_stats(&samples),
            Err(error) => {
                tracing::warn!(?error, "持股填息的同組基準查詢失敗，peer 留空");
                Vec::new()
            }
        };
    Json(response(member_id, days, outcomes, &peers)).into_response()
}

/// 會員目前持股的代號與名稱（`member_id` 為 0 時是全部會員合計）。
async fn fetch_holdings(pool: &PgPool, member_id: i64) -> sqlx::Result<Vec<(String, String)>> {
    sqlx::query_as(HOLDINGS_SQL)
        .bind(member_id)
        .fetch_all(pool)
        .await
}

/// 每檔持股除權息日在 `today` 往前 `days` 天內的填息結果，附代號、名稱與近 5 年的填息統計。
///
/// 每檔只查一次近 5 年的除權息與日線，回看期間的事件從中篩出（查詢次數與只看回看期間相同）。
async fn collect(
    repository: &impl DividendFillQuery,
    holdings: &[(String, String)],
    today: NaiveDate,
    days: u16,
) -> Result<Vec<HoldingFill>> {
    let since = today - Days::new(u64::from(days));
    let history_since = today - Days::new(HISTORY_DAYS);
    let mut collected = Vec::with_capacity(holdings.len());
    for (symbol, name) in holdings {
        let outcomes = fill::recent_outcomes(repository, symbol, history_since, today).await?;
        let history = fill::summarize(&outcomes);
        let recent = outcomes
            .into_iter()
            .filter(|outcome| outcome.event.ex_date > since)
            .collect();
        collected.push(HoldingFill {
            symbol: symbol.clone(),
            name: name.clone(),
            recent,
            history,
        });
    }
    Ok(collected)
}

/// 組出回應：未填息的依缺口由大到小在前，已填息的依除權息日由新到舊在後；`peers` 是各殖利率組的
/// 全市場基準（空的時候每列 `peer` 為 `null`）。
fn response(
    member_id: i64,
    days: u16,
    holdings: Vec<HoldingFill>,
    peers: &[BucketStats],
) -> PortfolioDividendFillResponse {
    let mut outcomes: Vec<(String, String, FillOutcome, FillSummary)> = Vec::new();
    for holding in holdings {
        for outcome in holding.recent {
            outcomes.push((
                holding.symbol.clone(),
                holding.name.clone(),
                outcome,
                holding.history,
            ));
        }
    }
    outcomes.sort_by(|(a_symbol, _, a, _), (b_symbol, _, b, _)| {
        a.fill_date
            .is_some()
            .cmp(&b.fill_date.is_some())
            .then_with(|| b.gap_percent().cmp(&a.gap_percent()))
            .then_with(|| Reverse(a.event.ex_date).cmp(&Reverse(b.event.ex_date)))
            .then_with(|| a_symbol.cmp(b_symbol))
    });
    let all: Vec<FillOutcome> = outcomes.iter().map(|(_, _, outcome, _)| *outcome).collect();
    PortfolioDividendFillResponse {
        member_id,
        days,
        summary: summary_dto(&fill::summarize(&all)),
        items: outcomes
            .iter()
            .map(
                |(symbol, name, outcome, history)| PortfolioDividendFillItem {
                    stock_symbol: symbol.clone(),
                    name: name.clone(),
                    fill: into_dto(outcome, symbol),
                    history: summary_dto(history),
                    peer: peer_of(outcome, peers).map(bucket_dto),
                },
            )
            .collect(),
    }
}

/// 這次除權息的現金殖利率所屬組別的基準；沒有現金股利或找不到該組時為 `None`。
fn peer_of<'a>(outcome: &FillOutcome, peers: &'a [BucketStats]) -> Option<&'a BucketStats> {
    let bucket = YieldBucket::of(outcome.event.cash_dividend, outcome.pre_ex_close)?;
    peers.iter().find(|stats| stats.bucket == bucket)
}

#[cfg(test)]
mod tests;
