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
use rust_decimal::Decimal;

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
  AND (($3 = 0 AND s.stock_exchange_market_id IN (2, 4)) OR s.stock_exchange_market_id = $3)
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
        median_change_percent: decimal_to_f64(peer::median(&values).map(|v| v.round_dp(2))),
        cap_weighted_change_percent: decimal_to_f64(cap_weighted.map(|v| v.round_dp(2))),
        trade_value: round2(trade_value),
        trade_value_share_percent: (total_value > Decimal::ZERO)
            .then(|| round2(trade_value / total_value * Decimal::ONE_HUNDRED)),
        median_price_earning_ratio: decimal_to_f64(
            peer::median(&positive(|row| row.price_earning_ratio)).map(|v| v.round_dp(2)),
        ),
        median_price_to_book_ratio: decimal_to_f64(
            peer::median(&positive(|row| row.price_to_book_ratio)).map(|v| v.round_dp(2)),
        ),
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

/// 四捨五入到小數 2 位的 JSON number。
fn round2(value: Decimal) -> f64 {
    decimal_to_f64(Some(value.round_dp(2))).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn row(
        industry_id: i32,
        symbol: &str,
        close: Decimal,
        change: Decimal,
        base_close: Option<Decimal>,
    ) -> IndustryRow {
        IndustryRow {
            industry_id,
            industry_name: format!("產業{industry_id}"),
            stock_symbol: symbol.to_string(),
            name: format!("股{symbol}"),
            closing_price: close,
            change_range: change,
            trade_value: dec!(100),
            price_earning_ratio: dec!(10),
            price_to_book_ratio: dec!(0),
            issued_share: 1000,
            base_close,
        }
    }

    /// 單日用交易所漲跌幅；區間用期初收盤價，期初沒有報價或為 0 時不列入。
    #[test]
    fn change_percent_uses_exchange_range_for_one_day_and_base_close_otherwise() {
        let r = row(1, "1101", dec!(110), dec!(-1.5), Some(dec!(100)));
        assert_eq!(change_percent(&r, 1), Some(dec!(-1.5)));
        assert_eq!(change_percent(&r, 5), Some(dec!(10)));
        assert_eq!(
            change_percent(&row(1, "1102", dec!(10), dec!(1), None), 5),
            None
        );
        assert_eq!(
            change_percent(&row(1, "1103", dec!(10), dec!(1), Some(dec!(0))), 5),
            None
        );
    }

    /// 市值加權：大型股主導方向；沒有發行股數的個股不列入，全部沒有時為 None。
    #[test]
    fn cap_weighted_change_weights_by_base_market_cap() {
        let big = IndustryRow {
            issued_share: 9000,
            ..row(1, "2330", dec!(110), dec!(10), None)
        };
        let small = row(1, "2303", dec!(90), dec!(-10), None);
        let no_shares = IndustryRow {
            issued_share: 0,
            ..row(1, "2302", dec!(50), dec!(50), None)
        };
        // 期初市值 900000 + 100000，期末 990000 + 90000 → +8%
        let changes = [
            (&big, dec!(10)),
            (&small, dec!(-10)),
            (&no_shares, dec!(50)),
        ];
        assert_eq!(
            cap_weighted_change(&changes).map(|v| v.round_dp(4)),
            Some(dec!(8))
        );
        assert_eq!(cap_weighted_change(&[(&no_shares, dec!(50))]), None);
        assert_eq!(cap_weighted_change(&[]), None);
    }

    /// 彙總：漲跌家數、中位數、成交值占比、估值只算正值、最大漲跌個股，並依加權漲跌幅排序。
    #[test]
    fn summarize_groups_by_industry_and_sorts_by_cap_weighted_change() {
        let mut flat = row(2, "1216", dec!(50), dec!(0), None);
        flat.trade_value = dec!(200);
        let rows = vec![
            row(1, "2330", dec!(110), dec!(3), None),
            row(1, "2303", dec!(50), dec!(-2), None),
            row(1, "2454", dec!(100), dec!(3), None),
            flat,
            IndustryRow {
                issued_share: 0,
                ..row(3, "9999", dec!(10), dec!(-1), None)
            },
        ];

        let industries = summarize(&rows, 1);
        let ids: Vec<i32> = industries.iter().map(|item| item.industry_id).collect();
        assert_eq!(ids, vec![1, 2, 3], "加權漲跌幅 None 的產業排最後");

        let semis = &industries[0];
        assert_eq!(semis.industry_name, "產業1");
        assert_eq!(
            (
                semis.stocks,
                semis.advancers,
                semis.decliners,
                semis.unchanged
            ),
            (3, 2, 1, 0)
        );
        assert_eq!(semis.median_change_percent, Some(3.0));
        assert_eq!(semis.trade_value, 300.0);
        assert_eq!(semis.trade_value_share_percent, Some(50.0));
        assert_eq!(semis.median_price_earning_ratio, Some(10.0));
        assert_eq!(semis.median_price_to_book_ratio, None, "淨值比 0 不列入");
        let gainer = semis.top_gainer.as_ref().unwrap();
        assert_eq!(
            (gainer.stock_symbol.as_str(), gainer.change_percent),
            ("2330", 3.0),
            "同漲幅取代號較小者"
        );
        assert_eq!(semis.top_loser.as_ref().unwrap().stock_symbol, "2303");

        let food = &industries[1];
        assert_eq!(
            (food.unchanged, food.cap_weighted_change_percent),
            (1, Some(0.0))
        );
        assert!(food.top_gainer.is_none() && food.top_loser.is_none());
        assert_eq!(industries[2].cap_weighted_change_percent, None);
    }

    /// 區間模式下期初沒有報價的個股只算檔數與成交值，不列入漲跌統計。
    #[test]
    fn summarize_skips_stocks_without_base_close_in_period_mode() {
        let rows = vec![
            row(1, "2330", dec!(110), dec!(0), Some(dec!(100))),
            row(1, "6999", dec!(50), dec!(9), None),
        ];
        let semis = &summarize(&rows, 20)[0];
        assert_eq!((semis.stocks, semis.advancers, semis.decliners), (2, 1, 0));
        assert_eq!(semis.median_change_percent, Some(10.0));
        assert_eq!(semis.cap_weighted_change_percent, Some(10.0));
    }

    /// 跌幅相同時也取代號較小者。
    #[test]
    fn top_loser_ties_pick_smaller_symbol() {
        let rows = vec![
            row(1, "2303", dec!(10), dec!(-2), None),
            row(1, "2301", dec!(10), dec!(-2), None),
        ];
        let loser = summarize(&rows, 1)[0].top_loser.clone().unwrap();
        assert_eq!(
            (loser.stock_symbol.as_str(), loser.change_percent),
            ("2301", -2.0)
        );
    }

    /// 全市場沒有成交金額時不計算占比。
    #[test]
    fn summarize_without_trade_value_has_no_share() {
        let mut quiet = row(1, "2330", dec!(10), dec!(1), None);
        quiet.trade_value = Decimal::ZERO;
        assert_eq!(summarize(&[quiet], 1)[0].trade_value_share_percent, None);
    }
}
