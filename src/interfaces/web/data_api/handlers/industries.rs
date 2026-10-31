//! `/market/industries` handler：各產業在指定交易日或區間的漲跌、成交值占比與估值中位數。
//!
//! 交易日直接取自 `"DailyQuotes"`（統計表 `daily_stock_price_stats` 只從 2022 年開始且有缺日）；
//! 區間起點用遞迴查詢逐日往前跳，每一步都只走 `"Date"` 索引。只算上市櫃普通股（排除
//! `00` 開頭的 ETF），當日沒有成交的股票不列入。

use std::collections::BTreeMap;

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;
use rust_decimal::{Decimal, RoundingStrategy};

use super::{
    database_error, decimal_to_f64, error_response, market_id_for_stocks, parse_optional_date,
};
use crate::domain::quote::peer;
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, IndustryMover, IndustryPerformance, IndustryPerformanceParams,
    IndustryPerformanceResponse,
};

/// 資料日（`$1` 之前最近的交易日）與往前 `$2` 個交易日的區間起點。
const DATES_SQL: &str = r#"
WITH RECURSIVE d(day, n) AS (
    SELECT (SELECT MAX("Date") FROM "DailyQuotes" WHERE $1::date IS NULL OR "Date" <= $1), 0
    UNION ALL
    SELECT (SELECT MAX("Date") FROM "DailyQuotes" WHERE "Date" < d.day), d.n + 1
    FROM d
    WHERE d.n < $2 AND d.day IS NOT NULL
)
SELECT
    (SELECT day FROM d WHERE n = 0) AS data_date,
    (SELECT day FROM d WHERE n = $2) AS base_date
"#;

/// 資料日有成交的上市櫃普通股，附區間起點收盤價。`$3 = 0` 是 all 的哨兵。
const ROWS_SQL: &str = r#"
SELECT
    s.stock_industry_id AS industry_id,
    COALESCE(si.name, '') AS industry_name,
    s.stock_symbol,
    s."Name" AS name,
    e."ClosingPrice" AS closing_price,
    e."ChangeRange" AS change_range,
    e."TradeValue" AS trade_value,
    e."PriceEarningRatio" AS price_earning_ratio,
    e."price-to-book_ratio" AS price_to_book_ratio,
    s.issued_share,
    b."ClosingPrice" AS base_close
FROM "DailyQuotes" e
JOIN stocks s ON s.stock_symbol = e.stock_symbol
LEFT JOIN stock_industry si ON si.stock_industry_id = s.stock_industry_id
LEFT JOIN "DailyQuotes" b ON b.stock_symbol = e.stock_symbol AND b."Date" = $2
WHERE e."Date" = $1
  AND e."ClosingPrice" > 0
  AND (($3 = 0 AND s.stock_exchange_market_id IN (2, 4)) OR ($3 <> 0 AND s.stock_exchange_market_id = $3))
  AND s.stock_symbol !~ '^00'
"#;

/// 單一個股在資料日的報價。
#[derive(Debug, Clone, sqlx::FromRow)]
struct IndustryRow {
    industry_id: i32,
    industry_name: String,
    stock_symbol: String,
    name: String,
    closing_price: Decimal,
    change_range: Decimal,
    trade_value: Decimal,
    price_earning_ratio: Decimal,
    price_to_book_ratio: Decimal,
    issued_share: i64,
    base_close: Option<Decimal>,
}

/// 查詢各產業的漲跌表現。
///
/// # Errors
///
/// 日期格式、`market` 或 `days`（1–60）不合法回 422；查無任何交易日回 404；驗證失敗回 401；
/// 資料庫查詢失敗時記錄內部錯誤並回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/industries", tag = "data-api", params(IndustryPerformanceParams), responses((status = 200, body = IndustryPerformanceResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn industry_performance(
    Query(params): Query<IndustryPerformanceParams>,
) -> Response {
    let date = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let market = params.market.as_deref().unwrap_or("all");
    let Some(market_id) = market_id_for_stocks(market) else {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "market 必須為 all、twse 或 tpex",
        );
    };
    let days = params.days.unwrap_or(1);
    if !(1..=60).contains(&days) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "days 必須介於 1 至 60");
    }

    let pool = database::get_connection();
    let dates: (Option<NaiveDate>, Option<NaiveDate>) = match sqlx::query_as(DATES_SQL)
        .bind(date)
        .bind(i32::from(days))
        .fetch_one(pool)
        .await
    {
        Ok(dates) => dates,
        Err(error) => return database_error(error),
    };
    let (Some(data_date), base_date) = dates else {
        return error_response(StatusCode::NOT_FOUND, "查無產業表現資料");
    };

    let rows: Vec<IndustryRow> = match sqlx::query_as(ROWS_SQL)
        .bind(data_date)
        .bind(base_date)
        .bind(market_id)
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };

    Json(IndustryPerformanceResponse {
        data_as_of: data_date.to_string(),
        base_date: base_date.map(|day| day.to_string()),
        market: market.to_string(),
        days,
        industries: summarize(&rows, days),
    })
    .into_response()
}

/// 個股的漲跌幅（%）：單日用交易所的漲跌幅（以參考價計算，已處理除權息），
/// 區間用期初收盤價；期初沒有報價（區間內新上市）時為 `None`。
fn change_percent(row: &IndustryRow, days: u8) -> Option<Decimal> {
    if days == 1 {
        return Some(row.change_range);
    }
    let base = row.base_close.filter(|price| *price > Decimal::ZERO)?;
    Some((row.closing_price / base - Decimal::ONE) * Decimal::ONE_HUNDRED)
}

/// 依產業彙總，並依市值加權漲跌幅由高到低排序（沒有值的排最後，同值依產業 id）。
fn summarize(rows: &[IndustryRow], days: u8) -> Vec<IndustryPerformance> {
    let total_value: Decimal = rows.iter().map(|row| row.trade_value).sum();
    let mut groups: BTreeMap<i32, Vec<&IndustryRow>> = BTreeMap::new();
    for row in rows {
        groups.entry(row.industry_id).or_default().push(row);
    }

    let mut industries: Vec<(Option<Decimal>, IndustryPerformance)> = groups
        .into_values()
        .map(|group| summarize_industry(&group, days, total_value))
        .collect();
    industries.sort_by(|(left, a), (right, b)| {
        right
            .cmp(left)
            .then_with(|| a.industry_id.cmp(&b.industry_id))
    });
    industries.into_iter().map(|(_, item)| item).collect()
}

/// 彙總單一產業；回傳的 `Decimal` 是排序用的市值加權漲跌幅。
fn summarize_industry(
    group: &[&IndustryRow],
    days: u8,
    total_value: Decimal,
) -> (Option<Decimal>, IndustryPerformance) {
    let changes: Vec<(&IndustryRow, Decimal)> = group
        .iter()
        .filter_map(|row| change_percent(row, days).map(|change| (*row, change)))
        .collect();
    let values: Vec<Decimal> = changes.iter().map(|(_, change)| *change).collect();
    let trade_value: Decimal = group.iter().map(|row| row.trade_value).sum();
    let positive = |pick: fn(&IndustryRow) -> Decimal| -> Vec<Decimal> {
        group
            .iter()
            .map(|row| pick(row))
            .filter(|value| *value > Decimal::ZERO)
            .collect()
    };
    let cap_weighted = cap_weighted_change(&changes);
    let mover = |found: Option<&(&IndustryRow, Decimal)>| {
        found.map(|(row, change)| IndustryMover {
            stock_symbol: row.stock_symbol.clone(),
            name: row.name.clone(),
            change_percent: round2(*change),
        })
    };
    // 同值時都取代號較小者，讓結果與資料庫回傳順序無關。
    let top_gainer = changes
        .iter()
        .filter(|(_, change)| *change > Decimal::ZERO)
        .max_by(|a, b| {
            a.1.cmp(&b.1)
                .then_with(|| b.0.stock_symbol.cmp(&a.0.stock_symbol))
        });
    let top_loser = changes
        .iter()
        .filter(|(_, change)| *change < Decimal::ZERO)
        .min_by(|a, b| {
            a.1.cmp(&b.1)
                .then_with(|| a.0.stock_symbol.cmp(&b.0.stock_symbol))
        });

    let item = IndustryPerformance {
        industry_id: group[0].industry_id,
        industry_name: group[0].industry_name.clone(),
        stocks: group.len(),
        advancers: values.iter().filter(|v| **v > Decimal::ZERO).count(),
        decliners: values.iter().filter(|v| **v < Decimal::ZERO).count(),
        unchanged: values.iter().filter(|v| v.is_zero()).count(),
        median_change_percent: peer::median(&values).map(round2),
        cap_weighted_change_percent: cap_weighted.map(round2),
        trade_value: round2(trade_value),
        trade_value_share_percent: (total_value > Decimal::ZERO)
            .then(|| round2(trade_value / total_value * Decimal::ONE_HUNDRED)),
        median_price_earning_ratio: peer::median(&positive(|row| row.price_earning_ratio))
            .map(round2),
        median_price_to_book_ratio: peer::median(&positive(|row| row.price_to_book_ratio))
            .map(round2),
        top_gainer: mover(top_gainer),
        top_loser: mover(top_loser),
    };
    (cap_weighted, item)
}

/// 以期初市值加權的漲跌幅（%）：Σ期末市值 ÷ Σ期初市值 − 1，期初市值＝期末市值 ÷ (1 + 漲跌幅)。
///
/// 沒有發行股數的個股不列入；全部都沒有時回 `None`。
fn cap_weighted_change(changes: &[(&IndustryRow, Decimal)]) -> Option<Decimal> {
    let mut end_total = Decimal::ZERO;
    let mut base_total = Decimal::ZERO;
    for (row, change) in changes {
        let growth = Decimal::ONE + *change / Decimal::ONE_HUNDRED;
        if row.issued_share <= 0 || growth <= Decimal::ZERO {
            continue;
        }
        let end_cap = row.closing_price * Decimal::from(row.issued_share);
        end_total += end_cap;
        base_total += end_cap / growth;
    }
    (base_total > Decimal::ZERO)
        .then(|| (end_total / base_total - Decimal::ONE) * Decimal::ONE_HUNDRED)
}

/// 四捨五入（不是 `round_dp` 預設的銀行家捨入）到小數 2 位的 JSON number。
fn round2(value: Decimal) -> f64 {
    decimal_to_f64(Some(
        value.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero),
    ))
    .unwrap_or_default()
}

#[cfg(test)]
mod tests;
