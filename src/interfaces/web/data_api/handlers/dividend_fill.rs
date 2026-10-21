//! `/stocks/{symbol}/dividend-fill` handler：最近幾次除權息的填息天數。
//!
//! 除權息事件與收盤價由 [`PgDividendFillRepository`] 讀取（與持股填息通知共用）；計算規則在
//! [`crate::domain::dividend::fill`]。

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Days, NaiveDate};
use rust_decimal::Decimal;

use super::{analytical_decimal_to_f64, ensure_stock_exists, error_response, repository_error};
use crate::domain::dividend::fill::{self, DividendFillQuery, ExRightEvent, FillOutcome};
use crate::infra::database::repository::dividend_fill::PgDividendFillRepository;
use crate::interfaces::web::data_api::dto::{
    DividendFillEvent, DividendFillParams, DividendFillResponse, DividendFillSummary, ErrorBody,
};

/// 查詢單一股票最近幾次除權息的填息結果與統計。
///
/// 只看最新收盤日（含）以前的除權息；除權息前後任一側沒有收盤的事件略過。
///
/// # Errors
///
/// `limit` 不在 1–40 回 422、股票不存在回 404、驗證失敗回 401；資料庫查詢失敗時
/// 記錄內部錯誤並回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/dividend-fill", tag = "data-api", params(("symbol" = String, Path, description = "股票代號"), DividendFillParams), responses((status = 200, body = DividendFillResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn dividend_fill(
    Path(symbol): Path<String>,
    Query(params): Query<DividendFillParams>,
) -> Response {
    let limit = params.limit.unwrap_or(12);
    if !(1..=40).contains(&limit) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 40");
    }
    if let Some(response) = ensure_stock_exists(&symbol).await {
        return response;
    }
    let repository = PgDividendFillRepository::new();

    let latest_quote = match repository.fetch_latest_quote_date(&symbol).await {
        Ok(date) => date,
        Err(error) => return repository_error(error),
    };
    let events = match repository.fetch_ex_right_events(&symbol).await {
        Ok(events) => select_events(events, latest_quote, usize::from(limit)),
        Err(error) => return repository_error(error),
    };

    let outcomes: Vec<FillOutcome> = match events.last() {
        Some(oldest) => {
            let since = oldest.ex_date - Days::new(fill::PRE_EX_LOOKBACK_DAYS);
            let closes = match repository.fetch_daily_closes(&symbol, since).await {
                Ok(closes) => closes,
                Err(error) => return repository_error(error),
            };
            events
                .into_iter()
                .filter_map(|event| fill::fill_outcome(event, &closes))
                .collect()
        }
        None => Vec::new(),
    };

    let summary = fill::summarize(&outcomes);
    Json(DividendFillResponse {
        data_as_of: latest_quote.map(|date| date.to_string()),
        summary: DividendFillSummary {
            events: summary.events,
            filled: summary.filled,
            average_trading_days_to_fill: summary.average_trading_days,
            median_trading_days_to_fill: summary.median_trading_days,
        },
        events: outcomes
            .iter()
            .map(|outcome| into_dto(outcome, &symbol))
            .collect(),
        stock_symbol: symbol,
    })
    .into_response()
}

/// 略過最新收盤日之後（尚未除權息）的事件，取最近 `limit` 次（由新到舊）。
fn select_events(
    events: Vec<ExRightEvent>,
    latest_quote: Option<NaiveDate>,
    limit: usize,
) -> Vec<ExRightEvent> {
    let Some(latest_quote) = latest_quote else {
        return Vec::new();
    };
    let mut events: Vec<ExRightEvent> = events
        .into_iter()
        .filter(|event| event.ex_date <= latest_quote)
        .collect();
    events.sort_by_key(|event| std::cmp::Reverse(event.ex_date));
    events.truncate(limit);
    events
}

pub(super) fn into_dto(outcome: &FillOutcome, symbol: &str) -> DividendFillEvent {
    let price =
        |value: Decimal, field: &'static str| analytical_decimal_to_f64(Some(value), symbol, field);
    DividendFillEvent {
        ex_date: outcome.event.ex_date.to_string(),
        cash_dividend: price(outcome.event.cash_dividend, "cash_dividend"),
        stock_dividend: price(outcome.event.stock_dividend, "stock_dividend"),
        pre_ex_date: outcome.pre_ex_date.to_string(),
        pre_ex_close: price(outcome.pre_ex_close, "pre_ex_close"),
        filled: outcome.fill_date.is_some(),
        fill_date: outcome.fill_date.map(|date| date.to_string()),
        trading_days_to_fill: outcome.trading_days_to_fill,
        trading_days_since_ex: outcome.trading_days_since_ex,
        latest_close: price(outcome.latest_close, "latest_close"),
        gap_percent: analytical_decimal_to_f64(outcome.gap_percent(), symbol, "gap_percent"),
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn row(ex_date: &str) -> ExRightEvent {
        ExRightEvent {
            ex_date: NaiveDate::parse_from_str(ex_date, "%Y-%m-%d").expect("測試日期合法"),
            cash_dividend: dec!(1),
            stock_dividend: Decimal::ZERO,
        }
    }

    /// 最新收盤日之後（尚未除息）的事件略過，其餘由新到舊取前 limit 筆；沒有日線時為空。
    #[test]
    fn select_events_filters_and_limits() {
        let latest = NaiveDate::from_ymd_opt(2026, 10, 8);
        let rows = || {
            vec![
                row("2025-06-12"),
                row("2026-12-10"),
                row("2026-09-16"),
                row("2026-06-11"),
            ]
        };
        let dates: Vec<String> = select_events(rows(), latest, 2)
            .iter()
            .map(|e| e.ex_date.to_string())
            .collect();
        assert_eq!(dates, ["2026-09-16", "2026-06-11"]);
        assert_eq!(select_events(rows(), latest, 40).len(), 3);
        assert!(select_events(rows(), None, 40).is_empty());
    }
}
