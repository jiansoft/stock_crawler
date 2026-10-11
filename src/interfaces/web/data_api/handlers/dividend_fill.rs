//! `/stocks/{symbol}/dividend-fill` handler：最近幾次除權息的填息天數。
//!
//! 除權息事件來自 `dividend`，收盤價來自 `DailyQuotes`；計算規則在
//! [`crate::domain::dividend::fill`]。

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Days, NaiveDate};
use rust_decimal::Decimal;

use super::{analytical_decimal_to_f64, database_error, ensure_stock_exists, error_response};
use crate::domain::dividend::fill::{self, DailyClose, ExRightEvent, FillOutcome};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    DividendFillEvent, DividendFillParams, DividendFillResponse, DividendFillSummary, ErrorBody,
};

/// 往除權息日前多抓幾天日線，確保長假或停牌後仍找得到除權息前一日的收盤。
const PRE_EX_LOOKBACK_DAYS: u64 = 30;

/// 每個除權息日一列。
///
/// 同一個除權息日可能有多列：季配股的年度彙總列、或同一期以季別與月別各記一次。
/// 每個日期只取一列，有各期列時不取彙總列（彙總列的金額是全年加總）。
const EX_RIGHT_EVENTS_SQL: &str = r#"
WITH cash AS (
    SELECT DISTINCT ON ("ex-dividend_date1") "ex-dividend_date1" AS ex_date, cash_dividend
    FROM dividend
    WHERE security_code = $1 AND "ex-dividend_date1" ~ '^\d{4}-\d{2}-\d{2}$' AND cash_dividend > 0
    ORDER BY "ex-dividend_date1", (quarter = ''), cash_dividend DESC
), stock AS (
    SELECT DISTINCT ON ("ex-dividend_date2") "ex-dividend_date2" AS ex_date, stock_dividend
    FROM dividend
    WHERE security_code = $1 AND "ex-dividend_date2" ~ '^\d{4}-\d{2}-\d{2}$' AND stock_dividend > 0
    ORDER BY "ex-dividend_date2", (quarter = ''), stock_dividend DESC
)
SELECT ex_date, COALESCE(cash.cash_dividend, 0) AS cash_dividend,
    COALESCE(stock.stock_dividend, 0) AS stock_dividend
FROM cash FULL JOIN stock USING (ex_date)
ORDER BY ex_date DESC
"#;

#[derive(sqlx::FromRow)]
struct EventRow {
    ex_date: String,
    cash_dividend: Decimal,
    stock_dividend: Decimal,
}

#[derive(sqlx::FromRow)]
struct CloseRow {
    #[sqlx(rename = "Date")]
    date: NaiveDate,
    #[sqlx(rename = "ClosingPrice")]
    close: Decimal,
}

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
    let pool = database::get_connection();

    let latest_quote: Option<NaiveDate> = match sqlx::query_scalar(
        r#"SELECT max("Date") FROM "DailyQuotes" WHERE stock_symbol = $1"#,
    )
    .bind(&symbol)
    .fetch_one(pool)
    .await
    {
        Ok(date) => date,
        Err(error) => return database_error(error),
    };
    let rows: Vec<EventRow> = match sqlx::query_as(EX_RIGHT_EVENTS_SQL)
        .bind(&symbol)
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    let events = select_events(rows, latest_quote, usize::from(limit));

    let outcomes: Vec<FillOutcome> = match events.last() {
        Some(oldest) => {
            let since = oldest.ex_date - Days::new(PRE_EX_LOOKBACK_DAYS);
            let closes: Vec<CloseRow> = match sqlx::query_as(
                r#"SELECT "Date", "ClosingPrice" FROM "DailyQuotes" WHERE stock_symbol = $1 AND "Date" >= $2 ORDER BY "Date""#,
            )
            .bind(&symbol)
            .bind(since)
            .fetch_all(pool)
            .await
            {
                Ok(rows) => rows,
                Err(error) => return database_error(error),
            };
            let closes: Vec<DailyClose> = closes
                .into_iter()
                .map(|row| DailyClose {
                    date: row.date,
                    close: row.close,
                })
                .collect();
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

/// 解析除權息日、略過格式錯誤與最新收盤日之後的事件，取最近 `limit` 次（由新到舊）。
fn select_events(
    rows: Vec<EventRow>,
    latest_quote: Option<NaiveDate>,
    limit: usize,
) -> Vec<ExRightEvent> {
    let Some(latest_quote) = latest_quote else {
        return Vec::new();
    };
    let mut events: Vec<ExRightEvent> = rows
        .into_iter()
        .filter_map(|row| {
            let ex_date = NaiveDate::parse_from_str(&row.ex_date, "%Y-%m-%d").ok()?;
            (ex_date <= latest_quote).then_some(ExRightEvent {
                ex_date,
                cash_dividend: row.cash_dividend,
                stock_dividend: row.stock_dividend,
            })
        })
        .collect();
    events.sort_by_key(|event| std::cmp::Reverse(event.ex_date));
    events.truncate(limit);
    events
}

fn into_dto(outcome: &FillOutcome, symbol: &str) -> DividendFillEvent {
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

    fn row(ex_date: &str) -> EventRow {
        EventRow {
            ex_date: ex_date.to_owned(),
            cash_dividend: dec!(1),
            stock_dividend: Decimal::ZERO,
        }
    }

    /// 格式錯誤與最新收盤日之後（尚未除息）的事件略過，其餘由新到舊取前 limit 筆；沒有日線時為空。
    #[test]
    fn select_events_filters_and_limits() {
        let latest = NaiveDate::from_ymd_opt(2026, 10, 8);
        let rows = || {
            vec![
                row("2025-06-12"),
                row("2026-12-10"),
                row("2026-09-16"),
                row("2026-02-30"),
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
