//! `/market/valuation` handler：上市櫃本益比與股價淨值比中位數的走勢與歷史位置。
//!
//! 每個交易日取 `DailyQuotes` 的官方本益比（證交所／櫃買中心日資料）與股價淨值比，只計大於 0 的值
//! （虧損股沒有本益比、ETF 兩者都沒有），算全市場中位數；再看最新一天的中位數落在回看期間的哪個位置。
//! 樣本數不到期間每日樣本數中位數一半的日子（例如收盤資料寫入到一半）不列入，避免少數股票扭曲中位數。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Datelike, Local, Months, NaiveDate};

use super::{database_error, error_response, market_id_for_stocks, parse_optional_date, round2};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, MarketValuationParams, MarketValuationPoint, MarketValuationResponse,
    ValuationRatioStats,
};

/// `$2`～`$1` 每個交易日的本益比、股價淨值比中位數（只計大於 0 的值）；`$3` 市場 id（0 為上市加上櫃）。
const DAILY_MEDIANS_SQL: &str = r#"
SELECT q."Date" AS date,
       (PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY q."PriceEarningRatio")
           FILTER (WHERE q."PriceEarningRatio" > 0))::float8 AS median_pe,
       (PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY q."price-to-book_ratio")
           FILTER (WHERE q."price-to-book_ratio" > 0))::float8 AS median_pb,
       COUNT(*) FILTER (WHERE q."PriceEarningRatio" > 0) AS pe_samples,
       COUNT(*) FILTER (WHERE q."price-to-book_ratio" > 0) AS pb_samples
FROM "DailyQuotes" q
JOIN stocks s ON s.stock_symbol = q.stock_symbol
WHERE q."Date" BETWEEN $2 AND $1
  AND (($3::int = 0 AND s.stock_exchange_market_id IN (2, 4)) OR s.stock_exchange_market_id = $3)
  AND (q."PriceEarningRatio" > 0 OR q."price-to-book_ratio" > 0)
GROUP BY q."Date"
ORDER BY q."Date"
"#;

/// [`DAILY_MEDIANS_SQL`] 的一列。
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
struct DailyMedian {
    date: NaiveDate,
    median_pe: Option<f64>,
    median_pb: Option<f64>,
    pe_samples: i64,
    pb_samples: i64,
}

/// 查詢市場估值水位。
///
/// # Errors
///
/// 參數格式錯誤、`years` 超出 1–10 或 `market` 不合法回 422；期間內沒有估值資料回 404；驗證失敗回 401；
/// 資料庫查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/valuation", tag = "data-api", params(MarketValuationParams), responses((status = 200, body = MarketValuationResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn market_valuation(Query(params): Query<MarketValuationParams>) -> Response {
    let date = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let years = params.years.unwrap_or(5);
    if !(1..=10).contains(&years) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "years 必須介於 1 至 10");
    }
    let market = params.market.as_deref().unwrap_or("all");
    let Some(market_id) = market_id_for_stocks(market) else {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "market 必須為 all、twse 或 tpex",
        );
    };
    let until = date.unwrap_or_else(|| Local::now().date_naive());
    let since = until
        .checked_sub_months(Months::new(u32::from(years) * 12))
        .unwrap_or(NaiveDate::MIN);
    // 五年全市場約 200 萬筆日線，正式庫約 2 秒；用批次連線池（慢查詢門檻較高）。
    let rows: Vec<DailyMedian> = match sqlx::query_as(DAILY_MEDIANS_SQL)
        .bind(until)
        .bind(since)
        .bind(market_id)
        .fetch_all(database::get_batch_connection())
        .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    match build_response(complete_days(rows), years, market) {
        Some(response) => Json(response).into_response(),
        None => error_response(StatusCode::NOT_FOUND, "查無估值資料"),
    }
}

/// 去掉樣本數不到每日樣本數中位數一半的日子（資料寫入到一半或缺漏）。
fn complete_days(rows: Vec<DailyMedian>) -> Vec<DailyMedian> {
    let mut samples: Vec<i64> = rows
        .iter()
        .map(|row| row.pe_samples.max(row.pb_samples))
        .collect();
    if samples.is_empty() {
        return rows;
    }
    samples.sort_unstable();
    let typical = samples[samples.len() / 2];
    rows.into_iter()
        .filter(|row| row.pe_samples.max(row.pb_samples) * 2 >= typical)
        .collect()
}

/// 由每日中位數（由舊到新）組出回應；沒有任何一天時回 `None`。
fn build_response(
    rows: Vec<DailyMedian>,
    years: u8,
    market: &str,
) -> Option<MarketValuationResponse> {
    let first = rows.first()?.date;
    let latest = rows.last()?.clone();
    let pe = ratio_stats(
        rows.iter().filter_map(|row| row.median_pe).collect(),
        latest.median_pe,
    );
    let pb = ratio_stats(
        rows.iter().filter_map(|row| row.median_pb).collect(),
        latest.median_pb,
    );
    let mut monthly: Vec<MarketValuationPoint> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let month_end = rows.get(index + 1).is_none_or(|next| {
            (next.date.year(), next.date.month()) != (row.date.year(), row.date.month())
        });
        if month_end {
            monthly.push(point(row));
        }
    }
    Some(MarketValuationResponse {
        data_as_of: latest.date.to_string(),
        from: first.to_string(),
        years,
        market: market.to_owned(),
        current: point(&latest),
        pe,
        pb,
        monthly,
    })
}

/// 一天的中位數（小數兩位）與樣本數。
fn point(row: &DailyMedian) -> MarketValuationPoint {
    MarketValuationPoint {
        date: row.date.to_string(),
        median_pe: row.median_pe.map(round2),
        median_pb: row.median_pb.map(round2),
        pe_samples: row.pe_samples,
        pb_samples: row.pb_samples,
    }
}

/// 每日中位數的分布與 `current` 的百分位（不高於它的日子占比）；`current` 或資料為空時回 `None`。
fn ratio_stats(mut values: Vec<f64>, current: Option<f64>) -> Option<ValuationRatioStats> {
    let current = current?;
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let not_above = values.iter().filter(|value| **value <= current).count();
    Some(ValuationRatioStats {
        current: round2(current),
        percentile: round2(not_above as f64 / values.len() as f64 * 100.0),
        min: round2(values[0]),
        p25: round2(quantile(&values, 0.25)),
        median: round2(quantile(&values, 0.5)),
        p75: round2(quantile(&values, 0.75)),
        max: round2(values[values.len() - 1]),
        days: values.len(),
    })
}

/// 已排序資料的分位數，線性內插（同 PostgreSQL `percentile_cont`）。
fn quantile(sorted: &[f64], fraction: f64) -> f64 {
    let position = fraction * (sorted.len() - 1) as f64;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    sorted[lower] + (sorted[upper] - sorted[lower]) * (position - lower as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(date: (i32, u32, u32), pe: Option<f64>, samples: i64) -> DailyMedian {
        DailyMedian {
            date: NaiveDate::from_ymd_opt(date.0, date.1, date.2).expect("合法日期"),
            median_pe: pe,
            median_pb: pe.map(|value| value / 10.0),
            pe_samples: samples,
            pb_samples: samples,
        }
    }

    /// 分位數線性內插：與 percentile_cont 相同。
    #[test]
    fn quantile_interpolates_linearly() {
        let values = [10.0, 12.0, 14.0, 20.0];
        assert_eq!(quantile(&values, 0.0), 10.0);
        assert_eq!(quantile(&values, 0.5), 13.0);
        assert_eq!(quantile(&values, 0.25), 11.5);
        assert_eq!(quantile(&values, 1.0), 20.0);
        assert_eq!(quantile(&[7.0], 0.75), 7.0);
    }

    /// 百分位是不高於最新一天的日子占比；沒有最新值或沒有資料時不算。
    #[test]
    fn ratio_stats_reports_distribution_and_percentile() {
        let stats = ratio_stats(vec![14.0, 10.0, 20.0, 12.0], Some(14.0)).expect("有資料");
        assert_eq!(
            stats,
            ValuationRatioStats {
                current: 14.0,
                percentile: 75.0,
                min: 10.0,
                p25: 11.5,
                median: 13.0,
                p75: 15.5,
                max: 20.0,
                days: 4,
            }
        );
        assert!(ratio_stats(vec![10.0], None).is_none());
        assert!(ratio_stats(Vec::new(), Some(10.0)).is_none());
    }

    /// 樣本數不到典型值一半的日子被去掉；空資料原樣回傳。
    #[test]
    fn complete_days_drops_partial_days() {
        let rows = vec![
            row((2026, 1, 2), Some(15.0), 1_000),
            row((2026, 1, 5), Some(15.5), 990),
            row((2026, 1, 6), Some(40.0), 30),
        ];
        let kept = complete_days(rows);
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().all(|row| row.pe_samples >= 990));
        assert!(complete_days(Vec::new()).is_empty());
    }

    /// 回應取最新一天為 current、每月最後一個交易日為 monthly；沒有資料回 None。
    #[test]
    fn build_response_samples_month_ends() {
        let rows = vec![
            row((2026, 1, 29), Some(14.0), 100),
            row((2026, 1, 30), Some(15.0), 100),
            row((2026, 2, 2), Some(16.0), 100),
            row((2026, 2, 27), Some(13.0), 100),
            row((2026, 3, 2), None, 100),
        ];
        let response = build_response(rows, 5, "all").expect("有資料");
        assert_eq!(
            (response.from.as_str(), response.data_as_of.as_str()),
            ("2026-01-29", "2026-03-02")
        );
        assert_eq!(
            response
                .monthly
                .iter()
                .map(|point| point.date.as_str())
                .collect::<Vec<_>>(),
            ["2026-01-30", "2026-02-27", "2026-03-02"]
        );
        assert_eq!(response.current.median_pe, None);
        // 最新一天沒有本益比：pe 為 null；淨值比同樣沒有。
        assert!(response.pe.is_none() && response.pb.is_none());
        assert_eq!(response.monthly[0].median_pb, Some(1.5));
        assert!(build_response(Vec::new(), 5, "all").is_none());
    }
}
