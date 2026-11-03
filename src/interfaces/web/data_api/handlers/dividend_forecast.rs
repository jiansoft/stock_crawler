//! `/portfolio/dividend-forecast` handler：以目前持股預估未來幾個月的現金股利入帳。
//!
//! 持股取 `stock_ownership_details`（未賣出，依代號合併），配息取 `dividend` 近兩年的
//! 現金股利列，報價取 `last_daily_quotes`；已除息未發放的取已領股利明細（與
//! `/portfolio/dividend-income` 同源）。預估規則在
//! [`crate::domain::portfolio::dividend_forecast`]。

use std::collections::HashMap;

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Datelike, Days, Local, Months, NaiveDate};
use rust_decimal::Decimal;

use super::dividend_fill::summary_dto;
use super::market_dividend_fill::bucket_dto;
use super::{database_error, error_response, repository_error};
use crate::app::calculation::dividend_fill_hints::FillHints;
use crate::domain::portfolio::dividend_forecast::{self, DividendEvent, Entitlement, Holding};
use crate::domain::portfolio::repository::PortfolioRepository;
use crate::infra::database;
use crate::infra::database::repository::portfolio::PgPortfolioRepository;
use crate::interfaces::web::data_api::dto::{
    DividendForecastItem, DividendForecastParams, DividendForecastResponse, ErrorBody,
};

/// 目前持股（依代號合併）與最新收盤價；`$1 = 0` 時為全部會員。
pub(super) const HOLDINGS_SQL: &str = r#"
SELECT o.security_code AS stock_symbol, COALESCE(s."Name", '') AS name,
    SUM(o.share_quantity)::bigint AS shares,
    MAX(CASE WHEN q.closing_price > 0 THEN q.closing_price END) AS closing_price
FROM stock_ownership_details o
LEFT JOIN stocks s ON s.stock_symbol = o.security_code
LEFT JOIN last_daily_quotes q ON q.stock_symbol = o.security_code
WHERE NOT o.is_sold AND ($1::bigint = 0 OR o.member_id = $1)
GROUP BY o.security_code, s."Name"
HAVING SUM(o.share_quantity) > 0
ORDER BY o.security_code
"#;

/// 指定代號自 `$2` 年起的現金股利列；日期欄位不是 `YYYY-MM-DD`（`-`、`尚未公布`）時為 NULL。
const EVENTS_SQL: &str = r#"
SELECT security_code AS stock_symbol, year_of_dividend, quarter, cash_dividend,
    CASE WHEN "ex-dividend_date1" ~ '^\d{4}-\d{2}-\d{2}$' THEN "ex-dividend_date1"::date END AS ex_date,
    CASE WHEN payable_date1 ~ '^\d{4}-\d{2}-\d{2}$' THEN payable_date1::date END AS payable_date
FROM dividend
WHERE security_code = ANY($1) AND cash_dividend > 0 AND year >= $2
"#;

/// [`HOLDINGS_SQL`] 的一列；`/portfolio/upcoming-dividends` 也用同一個查詢取持股股數。
#[derive(sqlx::FromRow)]
pub(super) struct HoldingRow {
    pub(super) stock_symbol: String,
    name: String,
    pub(super) shares: i64,
    closing_price: Option<Decimal>,
}

#[derive(sqlx::FromRow)]
struct EventRow {
    stock_symbol: String,
    year_of_dividend: i32,
    quarter: String,
    cash_dividend: Decimal,
    ex_date: Option<NaiveDate>,
    payable_date: Option<NaiveDate>,
}

/// 以目前持股預估未來幾個月每月的現金股利入帳。
///
/// 已公告的配息直接列入；過去 12 個月的配息往後推一年估算（季配、月配金額用最近一期，
/// 月配另依配息間隔補上去年同期沒有的月份）。只含現金股利，
/// 不扣二代健保補充保費與稅；已除息未發放的用除息日持股，其餘假設目前持股持有到除息日。
///
/// # Errors
///
/// `member_id` 為負數或 `months` 不在 1–24 回 422、驗證失敗回 401；資料庫查詢失敗時
/// 記錄內部錯誤並回不含 SQL 細節的 500。沒有持股時回 200 與 0。
#[utoipa::path(get, path = "/api/v1/portfolio/dividend-forecast", tag = "data-api", params(DividendForecastParams), responses((status = 200, body = DividendForecastResponse), (status = 401, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn dividend_forecast(Query(params): Query<DividendForecastParams>) -> Response {
    let member_id = params.member_id.unwrap_or(0);
    if member_id < 0 {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "member_id 不可為負數");
    }
    let months = params.months.unwrap_or(12);
    if !(1..=24).contains(&months) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "months 必須介於 1 至 24");
    }
    let today = Local::now().date_naive();
    let pool = database::get_connection();

    let holdings: Vec<HoldingRow> = match sqlx::query_as(HOLDINGS_SQL)
        .bind(member_id)
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    let symbols: Vec<String> = holdings
        .iter()
        .map(|row| row.stock_symbol.clone())
        .collect();
    let events: Vec<EventRow> = match sqlx::query_as(EVENTS_SQL)
        .bind(&symbols)
        .bind(today.year() - 2)
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    let end = today.checked_add_months(Months::new(u32::from(months)));
    let allocations = PgPortfolioRepository::new()
        .fetch_payout_allocations(member_id, today + Days::new(1), end.unwrap_or(today), today)
        .await;
    let entitlements = match allocations {
        Ok(rows) => dividend_forecast::entitlements(&rows),
        Err(error) => return repository_error(error),
    };

    let closing_prices: HashMap<String, Decimal> = holdings
        .iter()
        .filter_map(|row| Some((row.stock_symbol.clone(), row.closing_price?)))
        .collect();
    let mut response = build_response(member_id, holdings, events, &entitlements, today, months);
    let hints = FillHints::load(&symbols, today).await;
    attach_fill_hints(&mut response.items, &hints, &closing_prices);
    Json(response).into_response()
}

/// 每筆預估附該股近 5 年填息；沒有自身紀錄時以每股現金股利 ÷ 最新收盤附同殖利率組基準。
fn attach_fill_hints(
    items: &mut [DividendForecastItem],
    hints: &FillHints,
    closing_prices: &HashMap<String, Decimal>,
) {
    for item in items {
        if let Some(history) = hints.history(&item.stock_symbol) {
            item.fill_history = Some(summary_dto(history));
            continue;
        }
        let cash = Decimal::try_from(item.cash_per_share).unwrap_or_default();
        item.fill_peer = closing_prices
            .get(&item.stock_symbol)
            .and_then(|price| hints.peer(cash, *price))
            .map(bucket_dto);
    }
}

/// 由查詢結果組出回應。
fn build_response(
    member_id: i64,
    holdings: Vec<HoldingRow>,
    events: Vec<EventRow>,
    entitlements: &[Entitlement],
    today: NaiveDate,
    months: u8,
) -> DividendForecastResponse {
    let holdings: Vec<Holding> = holdings
        .into_iter()
        .map(|row| Holding {
            stock_symbol: row.stock_symbol,
            name: row.name,
            shares: row.shares,
            closing_price: row.closing_price,
        })
        .collect();
    let events: Vec<DividendEvent> = events
        .into_iter()
        .map(|row| DividendEvent {
            stock_symbol: row.stock_symbol,
            year_of_dividend: row.year_of_dividend,
            quarter: row.quarter,
            cash_per_share: row.cash_dividend,
            ex_date: row.ex_date,
            payable_date: row.payable_date,
        })
        .collect();
    DividendForecastResponse::new(
        member_id,
        dividend_forecast::forecast(&holdings, &events, entitlements, today, u32::from(months)),
    )
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    /// 查詢列原樣轉入領域預估，依據轉成 snake_case 字串。
    #[test]
    fn build_response_maps_rows() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 12).expect("有效日期");
        let holdings = vec![HoldingRow {
            stock_symbol: "2330".to_owned(),
            name: "台積電".to_owned(),
            shares: 100,
            closing_price: Some(dec!(1000)),
        }];
        let events = vec![EventRow {
            stock_symbol: "2330".to_owned(),
            year_of_dividend: 2026,
            quarter: "Q2".to_owned(),
            cash_dividend: dec!(7),
            ex_date: NaiveDate::from_ymd_opt(2026, 12, 10),
            payable_date: NaiveDate::from_ymd_opt(2027, 1, 7),
        }];
        let response = build_response(1, holdings, events, &[], today, 6);
        assert_eq!((response.member_id, response.total), (1, 700.0));
        assert_eq!(response.items[0].basis, "announced");
        assert_eq!(response.items[0].pay_date, "2027-01-07");
        assert_eq!(response.forward_yield_percent, Some(0.7));
        assert_eq!(response.months.len(), 7);
    }

    /// 有自身紀錄附 fill_history；沒有時以每股現金股利 ÷ 最新收盤附同組基準；沒有收盤價時兩者皆空。
    #[test]
    fn attach_fill_hints_prefers_own_history() {
        use crate::domain::dividend::fill::FillSummary;
        use crate::domain::dividend::fill_stats::{FillSample, YieldBucket, bucket_stats};

        let item = |symbol: &str, cash: f64| DividendForecastItem {
            stock_symbol: symbol.to_owned(),
            name: symbol.to_owned(),
            shares: 1_000,
            ex_date: "2026-12-11".to_owned(),
            pay_date: "2027-01-08".to_owned(),
            cash_per_share: cash,
            amount: cash * 1_000.0,
            basis: "announced".to_owned(),
            entitled: false,
            fill_history: None,
            fill_peer: None,
        };
        let mut items = vec![item("2330", 5.0), item("1101", 1.0), item("9999", 1.0)];
        let hints = FillHints::for_test(
            HashMap::from([(
                "2330".to_owned(),
                FillSummary {
                    events: 4,
                    filled: 4,
                    average_trading_days: Some(2.5),
                    median_trading_days: Some(2.0),
                },
            )]),
            bucket_stats(&[FillSample {
                bucket: YieldBucket::From3To5,
                trading_days_to_fill: Some(13),
                trading_days_observed: 120,
            }]),
        );
        let prices = HashMap::from([("1101".to_owned(), dec!(25))]);
        attach_fill_hints(&mut items, &hints, &prices);

        assert_eq!(items[0].fill_history.as_ref().map(|h| h.filled), Some(4));
        assert!(items[0].fill_peer.is_none());
        assert_eq!(
            items[1]
                .fill_peer
                .as_ref()
                .map(|p| (p.bucket.as_str(), p.median_trading_days_to_fill)),
            Some(("from_3_to_5", Some(13.0))),
            "1 ÷ 25 = 4%"
        );
        assert!(
            items[2].fill_history.is_none() && items[2].fill_peer.is_none(),
            "沒有收盤價"
        );
    }
}
