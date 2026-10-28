//! `/market/signals/performance` handler：各技術訊號出現後的報酬統計與回應組裝。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;

use super::HORIZONS;
use super::benchmark::{Benchmark, benchmark_output, fetch_benchmark};
use super::rows::PerformanceRow;
use super::sql::{
    LATEST_SIGNAL_DATE_SQL, PERFORMANCE_BY_YEAR_SQL, PERFORMANCE_SQL,
    PERFORMANCE_WITH_CHIP_BY_YEAR_SQL, PERFORMANCE_WITH_CHIP_SQL,
};
use super::validation::{
    parse_group_by, require_indicator_for_long_period, resolve_period, validate_rs_range,
    validate_streaks,
};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, HorizonStat, PerformanceYear, SignalPerformanceItem, SignalPerformanceParams,
    SignalPerformanceResponse,
};
use crate::interfaces::web::data_api::handlers::{
    database_error, error_response, industry_error, market_id_for_stocks, parse_optional_date,
    round2, round4, signal_filter_error, signal_indicator_order, signal_kind_order,
};

/// 統計期間內各技術訊號出現後 5／20／60／120／240 個交易日的表現。
///
/// 報酬為基準日收盤到之後第 N 個交易日收盤的還原報酬（等同含息）；尚未滿 N 日的訊號不列入
/// 該天期的樣本。超額報酬以同一天同市場全部股票的平均報酬為基準。
///
/// # Errors
///
/// 日期格式錯誤、起日晚於迄日、期間超過 3,653 天（沒有指定 `indicator` 時 1,096 天）、
/// `kind`／`indicator` 不在固定清單或產業
/// 名稱不存在、`min_rs_score`／`max_rs_score` 不在 1–99 或下限大於上限回 422；迄日之前沒有任何
/// 訊號資料回 404；驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的 500。
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
    if let Err(message) = validate_rs_range(params.min_rs_score, params.max_rs_score) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, message);
    }
    if let Err(message) = validate_streaks(params.min_foreign_streak, params.min_trust_streak) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, message);
    }
    let by_year = match parse_group_by(params.group_by.as_deref()) {
        Ok(by_year) => by_year,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
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
    if let Err(message) = require_indicator_for_long_period(from, to, params.indicator.as_deref()) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, message);
    }

    let with_chip = params.min_foreign_streak.is_some() || params.min_trust_streak.is_some();
    let sql = match (with_chip, by_year) {
        (true, true) => PERFORMANCE_WITH_CHIP_BY_YEAR_SQL,
        (true, false) => PERFORMANCE_WITH_CHIP_SQL,
        (false, true) => PERFORMANCE_BY_YEAR_SQL,
        (false, false) => PERFORMANCE_SQL,
    };
    let mut query = sqlx::query_as(sql)
        .bind(from)
        .bind(to)
        .bind(market_id)
        .bind(params.indicator.as_deref())
        .bind(params.kind.as_deref())
        .bind(params.industry.as_deref())
        .bind(params.min_rs_score.map(i16::from))
        .bind(params.max_rs_score.map(i16::from));
    if with_chip {
        query = query
            .bind(params.min_foreign_streak.map(i64::from))
            .bind(params.min_trust_streak.map(i64::from));
    }
    // 不快取成具名 prepared statement：同一連線執行約 5 次後 PostgreSQL 會改用 generic plan，
    // 來源、種類、產業與相對強度的可選條件剪不掉（2026-10-28 正式庫實測近一年全市場 custom 3.4 秒、
    // generic 56 秒；只看 rsi 三年 56 ms → 0.7 秒）。每次重新規劃只多幾毫秒。
    let rows: Vec<PerformanceRow> = match query.persistent(false).fetch_all(pool).await {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    let benchmark = match fetch_benchmark(from, to, market_id, by_year).await {
        Ok(benchmark) => benchmark,
        Err(error) => return database_error(error),
    };

    Json(build_performance(
        from, to, market, &params, rows, &benchmark,
    ))
    .into_response()
}

/// 某來源與種類（`year` 不為 `None` 的列）的各年度統計，由舊到新。
pub(in crate::interfaces::web::data_api::handlers) fn performance_years(
    rows: &[PerformanceRow],
    indicator: &str,
    kind: &str,
) -> Vec<PerformanceYear> {
    let mut years: Vec<PerformanceYear> = rows
        .iter()
        .filter(|row| row.indicator == indicator && row.kind == kind)
        .filter_map(|row| {
            Some(PerformanceYear {
                year: row.year?,
                occurrences: row.occurrences,
                horizons: horizon_stats(row),
            })
        })
        .collect();
    years.sort_by_key(|year| year.year);
    years
}

/// 整段期間的那一列（`year` 為 `None`）；沒有事件時為全 0 的預設值。
pub(in crate::interfaces::web::data_api::handlers) fn total_row(
    rows: &[PerformanceRow],
) -> PerformanceRow {
    rows.iter()
        .find(|row| row.year.is_none())
        .cloned()
        .unwrap_or_default()
}

/// 一列的各天期統計：平均與中位數四捨五入到小數兩位、比例到四位。
pub(in crate::interfaces::web::data_api::handlers) fn horizon_stats(
    row: &PerformanceRow,
) -> Vec<HorizonStat> {
    HORIZONS
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
        .collect()
}

/// 組成回應：項目依來源、種類的固定順序；平均與中位數四捨五入到小數兩位、比例到四位。
pub(super) fn build_performance(
    from: NaiveDate,
    to: NaiveDate,
    market: &str,
    params: &SignalPerformanceParams,
    rows: Vec<PerformanceRow>,
    benchmark: &Benchmark,
) -> SignalPerformanceResponse {
    let by_year = benchmark.years.is_some();
    let mut totals: Vec<&PerformanceRow> = rows.iter().filter(|row| row.year.is_none()).collect();
    totals.sort_by_key(|row| {
        (
            signal_indicator_order(&row.indicator),
            signal_kind_order(&row.kind),
        )
    });
    let items = totals
        .into_iter()
        .map(|row| SignalPerformanceItem {
            indicator: row.indicator.clone(),
            kind: row.kind.clone(),
            occurrences: row.occurrences,
            horizons: horizon_stats(row),
            years: by_year.then(|| performance_years(&rows, &row.indicator, &row.kind)),
        })
        .collect();
    let (benchmark, benchmark_by_year) = benchmark_output(benchmark);

    SignalPerformanceResponse {
        from: from.to_string(),
        to: to.to_string(),
        market: market.to_owned(),
        kind: params.kind.clone(),
        indicator: params.indicator.clone(),
        industry: params.industry.clone(),
        min_rs_score: params.min_rs_score,
        max_rs_score: params.max_rs_score,
        min_foreign_streak: params.min_foreign_streak,
        min_trust_streak: params.min_trust_streak,
        group_by: by_year.then(|| "year".to_owned()),
        adjusted: true,
        benchmark,
        benchmark_by_year,
        items,
    }
}
