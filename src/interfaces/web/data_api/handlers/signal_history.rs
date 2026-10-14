//! `/market/signals/performance` 與 `/stocks/{symbol}/signals` handler：技術訊號的歷史表現。
//!
//! 訊號來自 `daily_technical_signal`，之後的表現來自 `daily_forward_return`（收盤後回填的
//! 5／20／60 個交易日還原報酬，見 [`crate::app::calculation::forward_return`]）。

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Local, NaiveDate};
use rust_decimal::Decimal;

use super::{
    database_error, decimal_to_f64, ensure_stock_exists, error_response, industry_error,
    market_id_for_stocks, parse_optional_date, signal_filter_error, signal_indicator_order,
    signal_kind_order,
};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    BenchmarkStat, ErrorBody, HorizonStat, SignalPerformanceItem, SignalPerformanceParams,
    SignalPerformanceResponse, StockSignalItem, StockSignalsParams, StockSignalsResponse,
};

/// 統計的天期（交易日數），與 `daily_forward_return` 的欄位對應。
const HORIZONS: [u32; 3] = [5, 20, 60];

/// 預設統計期間：迄日往前 364 天（約一年）。
const DEFAULT_PERIOD_DAYS: i64 = 364;

/// 統計期間上限（日曆天，含起訖）。
const MAX_PERIOD_DAYS: i64 = 1096;

/// `$1`（含）之前最近一個有訊號的交易日；`$1` 為 `NULL` 時取最新。
const LATEST_SIGNAL_DATE_SQL: &str = r#"
SELECT MAX(date) FROM daily_technical_signal WHERE $1::date IS NULL OR date <= $1
"#;

/// 各來源與種類的訊號在各天期的報酬統計。`$3 = 0` 是 all 的哨兵（訊號只算上市櫃）；
/// 大盤基準（`bench`）是同一天同市場全部股票的平均報酬，不受產業篩選影響。
const PERFORMANCE_SQL: &str = r#"
WITH sig AS (
    SELECT t.date, t.stock_symbol, t.indicator, t.kind
    FROM daily_technical_signal t
    JOIN stocks s ON s.stock_symbol = t.stock_symbol
    LEFT JOIN stock_industry si ON si.stock_industry_id = s.stock_industry_id
    WHERE t.date BETWEEN $1 AND $2
      AND ($3 = 0 OR s.stock_exchange_market_id = $3)
      AND ($4::text IS NULL OR t.indicator = $4)
      AND ($5::text IS NULL OR t.kind = $5)
      AND ($6::text IS NULL OR si.name = $6)
),
bench AS (
    SELECT r.date,
           AVG(r.return_5d) AS b5, AVG(r.return_20d) AS b20, AVG(r.return_60d) AS b60
    FROM daily_forward_return r
    JOIN stocks s ON s.stock_symbol = r.stock_symbol
    WHERE r.date BETWEEN $1 AND $2
      AND ($3 = 0 OR s.stock_exchange_market_id = $3)
    GROUP BY r.date
)
SELECT
    sig.indicator,
    sig.kind,
    COUNT(*) AS occurrences,
    COUNT(r.return_5d) AS n5,
    AVG(r.return_5d)::float8 AS avg5,
    PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY r.return_5d::float8) AS med5,
    AVG(CASE WHEN r.return_5d > 0 THEN 1.0 WHEN r.return_5d IS NOT NULL THEN 0.0 END)::float8 AS win5,
    AVG(r.return_5d - b.b5)::float8 AS excess5,
    AVG(CASE WHEN r.return_5d > b.b5 THEN 1.0 WHEN r.return_5d IS NOT NULL AND b.b5 IS NOT NULL THEN 0.0 END)::float8 AS beat5,
    COUNT(r.return_20d) AS n20,
    AVG(r.return_20d)::float8 AS avg20,
    PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY r.return_20d::float8) AS med20,
    AVG(CASE WHEN r.return_20d > 0 THEN 1.0 WHEN r.return_20d IS NOT NULL THEN 0.0 END)::float8 AS win20,
    AVG(r.return_20d - b.b20)::float8 AS excess20,
    AVG(CASE WHEN r.return_20d > b.b20 THEN 1.0 WHEN r.return_20d IS NOT NULL AND b.b20 IS NOT NULL THEN 0.0 END)::float8 AS beat20,
    COUNT(r.return_60d) AS n60,
    AVG(r.return_60d)::float8 AS avg60,
    PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY r.return_60d::float8) AS med60,
    AVG(CASE WHEN r.return_60d > 0 THEN 1.0 WHEN r.return_60d IS NOT NULL THEN 0.0 END)::float8 AS win60,
    AVG(r.return_60d - b.b60)::float8 AS excess60,
    AVG(CASE WHEN r.return_60d > b.b60 THEN 1.0 WHEN r.return_60d IS NOT NULL AND b.b60 IS NOT NULL THEN 0.0 END)::float8 AS beat60
FROM sig
LEFT JOIN daily_forward_return r ON r.date = sig.date AND r.stock_symbol = sig.stock_symbol
LEFT JOIN bench b ON b.date = sig.date
GROUP BY sig.indicator, sig.kind
"#;

/// 期間內同市場全部股票的報酬統計（大盤基準）。
const BENCHMARK_SQL: &str = r#"
SELECT
    COUNT(r.return_5d) AS n5,
    AVG(r.return_5d)::float8 AS avg5,
    AVG(CASE WHEN r.return_5d > 0 THEN 1.0 WHEN r.return_5d IS NOT NULL THEN 0.0 END)::float8 AS win5,
    COUNT(r.return_20d) AS n20,
    AVG(r.return_20d)::float8 AS avg20,
    AVG(CASE WHEN r.return_20d > 0 THEN 1.0 WHEN r.return_20d IS NOT NULL THEN 0.0 END)::float8 AS win20,
    COUNT(r.return_60d) AS n60,
    AVG(r.return_60d)::float8 AS avg60,
    AVG(CASE WHEN r.return_60d > 0 THEN 1.0 WHEN r.return_60d IS NOT NULL THEN 0.0 END)::float8 AS win60
FROM daily_forward_return r
JOIN stocks s ON s.stock_symbol = r.stock_symbol
WHERE r.date BETWEEN $1 AND $2
  AND ($3 = 0 OR s.stock_exchange_market_id = $3)
"#;

/// 個股在期間內的訊號，附當日報價與之後的報酬，由新到舊；`total` 為未套用 limit 的筆數。
const STOCK_SIGNALS_SQL: &str = r#"
SELECT
    t.date,
    t.indicator,
    t.kind,
    t.description,
    q."ClosingPrice" AS closing_price,
    q."ChangeRange" AS change_range,
    r.return_5d,
    r.return_20d,
    r.return_60d,
    COUNT(*) OVER () AS total
FROM daily_technical_signal t
LEFT JOIN "DailyQuotes" q ON q.stock_symbol = t.stock_symbol AND q."Date" = t.date
LEFT JOIN daily_forward_return r ON r.stock_symbol = t.stock_symbol AND r.date = t.date
WHERE t.stock_symbol = $1
  AND t.date BETWEEN $2 AND $3
  AND ($4::text IS NULL OR t.indicator = $4)
  AND ($5::text IS NULL OR t.kind = $5)
ORDER BY t.date DESC, t.indicator, t.kind
LIMIT $6
"#;

/// 單一天期的統計值。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct HorizonValues {
    samples: i64,
    avg: Option<f64>,
    median: Option<f64>,
    win: Option<f64>,
    excess: Option<f64>,
    beat: Option<f64>,
}

/// [`PERFORMANCE_SQL`] 的一列。
#[derive(Debug, Clone, sqlx::FromRow)]
struct PerformanceRow {
    indicator: String,
    kind: String,
    occurrences: i64,
    n5: i64,
    avg5: Option<f64>,
    med5: Option<f64>,
    win5: Option<f64>,
    excess5: Option<f64>,
    beat5: Option<f64>,
    n20: i64,
    avg20: Option<f64>,
    med20: Option<f64>,
    win20: Option<f64>,
    excess20: Option<f64>,
    beat20: Option<f64>,
    n60: i64,
    avg60: Option<f64>,
    med60: Option<f64>,
    win60: Option<f64>,
    excess60: Option<f64>,
    beat60: Option<f64>,
}

impl PerformanceRow {
    /// 依 [`HORIZONS`] 順序的各天期統計。
    fn horizons(&self) -> [HorizonValues; 3] {
        [
            HorizonValues {
                samples: self.n5,
                avg: self.avg5,
                median: self.med5,
                win: self.win5,
                excess: self.excess5,
                beat: self.beat5,
            },
            HorizonValues {
                samples: self.n20,
                avg: self.avg20,
                median: self.med20,
                win: self.win20,
                excess: self.excess20,
                beat: self.beat20,
            },
            HorizonValues {
                samples: self.n60,
                avg: self.avg60,
                median: self.med60,
                win: self.win60,
                excess: self.excess60,
                beat: self.beat60,
            },
        ]
    }
}

/// [`BENCHMARK_SQL`] 的結果。
#[derive(Debug, Clone, Default, sqlx::FromRow)]
struct BenchmarkRow {
    n5: i64,
    avg5: Option<f64>,
    win5: Option<f64>,
    n20: i64,
    avg20: Option<f64>,
    win20: Option<f64>,
    n60: i64,
    avg60: Option<f64>,
    win60: Option<f64>,
}

/// [`STOCK_SIGNALS_SQL`] 的一列。
#[derive(Debug, Clone, sqlx::FromRow)]
struct StockSignalRow {
    date: NaiveDate,
    indicator: String,
    kind: String,
    description: String,
    closing_price: Option<Decimal>,
    change_range: Option<Decimal>,
    return_5d: Option<Decimal>,
    return_20d: Option<Decimal>,
    return_60d: Option<Decimal>,
    total: i64,
}

/// 統計期間內各技術訊號出現後 5／20／60 個交易日的表現。
///
/// 報酬為基準日收盤到之後第 N 個交易日收盤的還原報酬（等同含息）；尚未滿 N 日的訊號不列入
/// 該天期的樣本。超額報酬以同一天同市場全部股票的平均報酬為基準。
///
/// # Errors
///
/// 日期格式錯誤、起日晚於迄日、期間超過 1,096 天、`kind`／`indicator` 不在固定清單或產業
/// 名稱不存在回 422；迄日之前沒有任何訊號資料回 404；驗證失敗回 401；資料庫查詢失敗回
/// 不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/signals/performance", tag = "data-api", params(SignalPerformanceParams), responses((status = 200, body = SignalPerformanceResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn signal_performance(Query(params): Query<SignalPerformanceParams>) -> Response {
    let (from, to) = match (
        parse_optional_date(params.from.as_deref()),
        parse_optional_date(params.to.as_deref()),
    ) {
        (Ok(from), Ok(to)) => (from, to),
        (Err(message), _) | (_, Err(message)) => {
            return error_response(StatusCode::UNPROCESSABLE_ENTITY, message);
        }
    };
    let market = params.market.as_deref().unwrap_or("all");
    let Some(market_id) = market_id_for_stocks(market) else {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "market 必須為 all、twse 或 tpex",
        );
    };
    if let Some(response) = signal_filter_error(params.kind.as_deref(), params.indicator.as_deref())
    {
        return response;
    }
    if let Some(response) = industry_error(params.industry.as_deref()).await {
        return response;
    }

    let pool = database::get_connection();
    let latest: Option<NaiveDate> = match sqlx::query_scalar(LATEST_SIGNAL_DATE_SQL)
        .bind(to)
        .fetch_one(pool)
        .await
    {
        Ok(value) => value,
        Err(error) => return database_error(error),
    };
    let Some(to) = latest else {
        return error_response(StatusCode::NOT_FOUND, "查無技術訊號資料");
    };
    let from = match resolve_period(from, to) {
        Ok(from) => from,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };

    let rows: Vec<PerformanceRow> = match sqlx::query_as(PERFORMANCE_SQL)
        .bind(from)
        .bind(to)
        .bind(market_id)
        .bind(params.indicator.as_deref())
        .bind(params.kind.as_deref())
        .bind(params.industry.as_deref())
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    let benchmark: BenchmarkRow = match sqlx::query_as(BENCHMARK_SQL)
        .bind(from)
        .bind(to)
        .bind(market_id)
        .fetch_one(pool)
        .await
    {
        Ok(row) => row,
        Err(error) => return database_error(error),
    };

    Json(build_performance(
        from, to, market, &params, rows, &benchmark,
    ))
    .into_response()
}

/// 起日：未指定為迄日往前 [`DEFAULT_PERIOD_DAYS`] 天；晚於迄日或期間超過上限回錯誤訊息。
fn resolve_period(from: Option<NaiveDate>, to: NaiveDate) -> Result<NaiveDate, &'static str> {
    let from = from.unwrap_or(to - chrono::Duration::days(DEFAULT_PERIOD_DAYS));
    if from > to {
        return Err("from 不可晚於 to");
    }
    if (to - from).num_days() >= MAX_PERIOD_DAYS {
        return Err("統計期間最長 1,096 天");
    }
    Ok(from)
}

/// 組成回應：項目依來源、種類的固定順序；平均與中位數四捨五入到小數兩位、比例到四位。
fn build_performance(
    from: NaiveDate,
    to: NaiveDate,
    market: &str,
    params: &SignalPerformanceParams,
    mut rows: Vec<PerformanceRow>,
    benchmark: &BenchmarkRow,
) -> SignalPerformanceResponse {
    rows.sort_by_key(|row| {
        (
            signal_indicator_order(&row.indicator),
            signal_kind_order(&row.kind),
        )
    });
    let items = rows
        .iter()
        .map(|row| SignalPerformanceItem {
            indicator: row.indicator.clone(),
            kind: row.kind.clone(),
            occurrences: row.occurrences,
            horizons: HORIZONS
                .iter()
                .zip(row.horizons())
                .map(|(&days, values)| HorizonStat {
                    days,
                    samples: values.samples,
                    avg_return: values.avg.map(round2),
                    median_return: values.median.map(round2),
                    win_rate: values.win.map(round4),
                    avg_excess_return: values.excess.map(round2),
                    beat_market_rate: values.beat.map(round4),
                })
                .collect(),
        })
        .collect();
    let benchmark = [
        (benchmark.n5, benchmark.avg5, benchmark.win5),
        (benchmark.n20, benchmark.avg20, benchmark.win20),
        (benchmark.n60, benchmark.avg60, benchmark.win60),
    ]
    .into_iter()
    .zip(HORIZONS)
    .map(|((samples, avg, win), days)| BenchmarkStat {
        days,
        samples,
        avg_return: avg.map(round2),
        win_rate: win.map(round4),
    })
    .collect();

    SignalPerformanceResponse {
        from: from.to_string(),
        to: to.to_string(),
        market: market.to_owned(),
        kind: params.kind.clone(),
        indicator: params.indicator.clone(),
        industry: params.industry.clone(),
        adjusted: true,
        benchmark,
        items,
    }
}

/// 查詢個股在期間內出現過的技術訊號，以及每次訊號之後 5／20／60 個交易日的還原報酬。
///
/// # Errors
///
/// 日期格式錯誤、`days` 不在 1–730、`limit` 不在 1–200 或 `kind`／`indicator` 不在固定清單回 422；
/// 股票不存在回 404；驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。期間內沒有訊號回空清單。
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/signals", tag = "data-api", params(("symbol" = String, Path, description = "股票代號"), StockSignalsParams), responses((status = 200, body = StockSignalsResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn stock_signals(
    Path(symbol): Path<String>,
    Query(params): Query<StockSignalsParams>,
) -> Response {
    let to = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value.unwrap_or_else(|| Local::now().date_naive()),
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let days = params.days.unwrap_or(180);
    if !(1..=730).contains(&days) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "days 必須介於 1 至 730");
    }
    let limit = params.limit.unwrap_or(50);
    if !(1..=200).contains(&limit) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 200");
    }
    if let Some(response) = signal_filter_error(params.kind.as_deref(), params.indicator.as_deref())
    {
        return response;
    }
    if let Some(response) = ensure_stock_exists(&symbol).await {
        return response;
    }

    let from = to - chrono::Duration::days(i64::from(days) - 1);
    let rows: Vec<StockSignalRow> = match sqlx::query_as(STOCK_SIGNALS_SQL)
        .bind(&symbol)
        .bind(from)
        .bind(to)
        .bind(params.indicator.as_deref())
        .bind(params.kind.as_deref())
        .bind(i64::from(limit))
        .fetch_all(database::get_connection())
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };

    Json(build_stock_signals(symbol, from, to, rows)).into_response()
}

/// 組成個股訊號歷史；同一天的訊號依來源、種類的固定順序。
fn build_stock_signals(
    symbol: String,
    from: NaiveDate,
    to: NaiveDate,
    mut rows: Vec<StockSignalRow>,
) -> StockSignalsResponse {
    let total = rows.first().map_or(0, |row| row.total);
    rows.sort_by(|a, b| {
        b.date
            .cmp(&a.date)
            .then_with(|| {
                signal_indicator_order(&a.indicator).cmp(&signal_indicator_order(&b.indicator))
            })
            .then_with(|| signal_kind_order(&a.kind).cmp(&signal_kind_order(&b.kind)))
    });
    StockSignalsResponse {
        stock_symbol: symbol,
        from: from.to_string(),
        to: to.to_string(),
        adjusted: true,
        total,
        items: rows
            .into_iter()
            .map(|row| StockSignalItem {
                date: row.date.to_string(),
                indicator: row.indicator,
                kind: row.kind,
                description: row.description,
                closing_price: decimal_to_f64(row.closing_price),
                change_percent: decimal_to_f64(row.change_range),
                return_5d: decimal_to_f64(row.return_5d),
                return_20d: decimal_to_f64(row.return_20d),
                return_60d: decimal_to_f64(row.return_60d),
            })
            .collect(),
    }
}

/// 四捨五入到小數兩位。
fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// 四捨五入到小數四位（比例）。
fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("有效日期")
    }

    fn performance_row(indicator: &str, kind: &str, n60: i64) -> PerformanceRow {
        PerformanceRow {
            indicator: indicator.to_owned(),
            kind: kind.to_owned(),
            occurrences: 10,
            n5: 10,
            avg5: Some(1.234_56),
            med5: Some(0.996),
            win5: Some(0.666_666),
            excess5: Some(-0.333_3),
            beat5: Some(0.5),
            n20: 8,
            avg20: Some(2.0),
            med20: None,
            win20: Some(0.75),
            excess20: None,
            beat20: None,
            n60,
            avg60: (n60 > 0).then_some(5.0),
            med60: None,
            win60: None,
            excess60: None,
            beat60: None,
        }
    }

    fn params() -> SignalPerformanceParams {
        SignalPerformanceParams {
            from: None,
            to: None,
            market: None,
            kind: Some("golden_cross".to_owned()),
            indicator: None,
            industry: Some("半導體業".to_owned()),
        }
    }

    /// 起日預設迄日往前 364 天；起日晚於迄日或期間超過 1,096 天回錯誤。
    #[test]
    fn resolve_period_defaults_and_limits() {
        let to = day(2026, 10, 13);
        assert_eq!(resolve_period(None, to), Ok(day(2025, 10, 14)));
        assert_eq!(resolve_period(Some(to), to), Ok(to));
        assert_eq!(
            resolve_period(Some(day(2023, 10, 14)), to),
            Ok(day(2023, 10, 14))
        );
        assert_eq!(
            resolve_period(Some(day(2023, 10, 13)), to),
            Err("統計期間最長 1,096 天")
        );
        assert_eq!(
            resolve_period(Some(day(2026, 10, 14)), to),
            Err("from 不可晚於 to")
        );
    }

    /// 項目依來源、種類的固定順序；數值四捨五入；天期與基準依 5、20、60 排列。
    #[test]
    fn build_performance_orders_and_rounds() {
        let rows = vec![
            performance_row("52_week", "new_high", 0),
            performance_row("kd", "oversold", 3),
            performance_row("kd", "golden_cross", 3),
            performance_row("ma5_ma20", "death_cross", 3),
        ];
        let benchmark = BenchmarkRow {
            n5: 100,
            avg5: Some(0.123_45),
            win5: Some(0.512_34),
            ..Default::default()
        };
        let response = build_performance(
            day(2025, 10, 14),
            day(2026, 10, 13),
            "all",
            &params(),
            rows,
            &benchmark,
        );

        let order: Vec<(&str, &str)> = response
            .items
            .iter()
            .map(|item| (item.indicator.as_str(), item.kind.as_str()))
            .collect();
        assert_eq!(
            order,
            [
                ("ma5_ma20", "death_cross"),
                ("kd", "golden_cross"),
                ("kd", "oversold"),
                ("52_week", "new_high"),
            ]
        );
        let first = &response.items[0].horizons;
        assert_eq!(
            first[0],
            HorizonStat {
                days: 5,
                samples: 10,
                avg_return: Some(1.23),
                median_return: Some(1.0),
                win_rate: Some(0.6667),
                avg_excess_return: Some(-0.33),
                beat_market_rate: Some(0.5),
            }
        );
        assert_eq!(first[1].days, 20);
        assert_eq!(first[1].median_return, None);
        let last = &response.items[3].horizons[2];
        assert_eq!((last.days, last.samples, last.avg_return), (60, 0, None));

        assert_eq!(
            response.benchmark,
            [
                BenchmarkStat {
                    days: 5,
                    samples: 100,
                    avg_return: Some(0.12),
                    win_rate: Some(0.5123),
                },
                BenchmarkStat {
                    days: 20,
                    samples: 0,
                    avg_return: None,
                    win_rate: None,
                },
                BenchmarkStat {
                    days: 60,
                    samples: 0,
                    avg_return: None,
                    win_rate: None,
                },
            ]
        );
        assert_eq!(response.from, "2025-10-14");
        assert_eq!(response.to, "2026-10-13");
        assert!(response.adjusted);
        assert_eq!(response.kind.as_deref(), Some("golden_cross"));
        assert_eq!(response.industry.as_deref(), Some("半導體業"));
        assert!(response.indicator.is_none());
    }

    fn stock_row(date: NaiveDate, indicator: &str, kind: &str) -> StockSignalRow {
        StockSignalRow {
            date,
            indicator: indicator.to_owned(),
            kind: kind.to_owned(),
            description: format!("{indicator} {kind}"),
            closing_price: Some(Decimal::new(1005, 1)),
            change_range: Some(Decimal::new(-123, 2)),
            return_5d: Some(Decimal::new(25, 1)),
            return_20d: None,
            return_60d: None,
            total: 7,
        }
    }

    /// 由新到舊；同一天依來源、種類的固定順序；total 取自查詢的視窗計數，空清單為 0。
    #[test]
    fn build_stock_signals_orders_items() {
        let rows = vec![
            stock_row(day(2026, 10, 12), "kd", "golden_cross"),
            stock_row(day(2026, 10, 13), "52_week", "new_high"),
            stock_row(day(2026, 10, 13), "ma5_ma20", "golden_cross"),
        ];
        let response =
            build_stock_signals("2330".to_owned(), day(2026, 4, 17), day(2026, 10, 13), rows);
        let order: Vec<(&str, &str)> = response
            .items
            .iter()
            .map(|item| (item.date.as_str(), item.indicator.as_str()))
            .collect();
        assert_eq!(
            order,
            [
                ("2026-10-13", "ma5_ma20"),
                ("2026-10-13", "52_week"),
                ("2026-10-12", "kd"),
            ]
        );
        assert_eq!(response.total, 7);
        assert!(response.adjusted);
        let first = &response.items[0];
        assert_eq!(first.closing_price, Some(100.5));
        assert_eq!(first.change_percent, Some(-1.23));
        assert_eq!(first.return_5d, Some(2.5));
        assert_eq!(first.return_20d, None);

        let empty =
            build_stock_signals("2330".to_owned(), day(2026, 1, 1), day(2026, 1, 2), vec![]);
        assert_eq!(empty.total, 0);
        assert!(empty.items.is_empty());
    }
}
