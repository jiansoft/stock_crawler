//! `/market/valuation` handler：上市櫃本益比與股價淨值比中位數的走勢與歷史位置。
//!
//! 每日中位數讀預先算好的 `daily_market_valuation`（官方本益比與股價淨值比，只計大於 0 的值）；
//! 樣本不足的日子與分布計算見 [`crate::domain::market_index::valuation`]。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Datelike, Local, Months, NaiveDate};

use super::{error_response, market_id_for_stocks, parse_optional_date, repository_error, round2};
use crate::domain::market_index::valuation::{
    MarketValuationDay, MarketValuationRepository, ValuationDistribution, complete_days,
    distribution,
};
use crate::infra::database::repository::market_valuation::PgMarketValuationRepository;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, MarketValuationParams, MarketValuationPoint, MarketValuationResponse,
    ValuationRatioStats,
};

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
    let days = match PgMarketValuationRepository::new()
        .fetch_market_valuation(market_id, since, until)
        .await
    {
        Ok(days) => days,
        Err(error) => return repository_error(error),
    };
    match build_response(complete_days(days), years, market) {
        Some(response) => Json(response).into_response(),
        None => error_response(StatusCode::NOT_FOUND, "查無估值資料"),
    }
}

/// 由每日中位數（由舊到新）組出回應；沒有任何一天時回 `None`。
fn build_response(
    days: Vec<MarketValuationDay>,
    years: u8,
    market: &str,
) -> Option<MarketValuationResponse> {
    let first = days.first()?.date;
    let latest = days.last()?.clone();
    let pe = distribution(
        days.iter().filter_map(|day| day.median_pe).collect(),
        latest.median_pe,
    );
    let pb = distribution(
        days.iter().filter_map(|day| day.median_pb).collect(),
        latest.median_pb,
    );
    let monthly = days
        .iter()
        .enumerate()
        .filter(|(index, day)| {
            days.get(index + 1).is_none_or(|next| {
                (next.date.year(), next.date.month()) != (day.date.year(), day.date.month())
            })
        })
        .map(|(_, day)| point(day))
        .collect();
    Some(MarketValuationResponse {
        data_as_of: latest.date.to_string(),
        from: first.to_string(),
        years,
        market: market.to_owned(),
        current: point(&latest),
        pe: pe.map(stats),
        pb: pb.map(stats),
        monthly,
    })
}

/// 一天的中位數（小數兩位）與樣本數。
fn point(day: &MarketValuationDay) -> MarketValuationPoint {
    MarketValuationPoint {
        date: day.date.to_string(),
        median_pe: day.median_pe.map(round2),
        median_pb: day.median_pb.map(round2),
        pe_samples: day.pe_samples,
        pb_samples: day.pb_samples,
    }
}

/// 分布轉成 API 契約（小數兩位）。
fn stats(distribution: ValuationDistribution) -> ValuationRatioStats {
    ValuationRatioStats {
        current: round2(distribution.current),
        percentile: round2(distribution.percentile),
        min: round2(distribution.min),
        p25: round2(distribution.p25),
        median: round2(distribution.median),
        p75: round2(distribution.p75),
        max: round2(distribution.max),
        days: distribution.days,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(date: (i32, u32, u32), pe: Option<f64>) -> MarketValuationDay {
        MarketValuationDay {
            date: NaiveDate::from_ymd_opt(date.0, date.1, date.2).expect("合法日期"),
            median_pe: pe,
            median_pb: pe.map(|value| value / 10.0),
            pe_samples: 100,
            pb_samples: 100,
        }
    }

    /// 回應取最新一天為 current、每月最後一個交易日為 monthly；數值四捨五入到小數兩位；沒有資料回 None。
    #[test]
    fn build_response_samples_month_ends() {
        let days = vec![
            day((2026, 1, 29), Some(14.0)),
            day((2026, 1, 30), Some(15.0)),
            day((2026, 2, 2), Some(16.0)),
            day((2026, 2, 27), Some(13.333)),
        ];
        let response = build_response(days.clone(), 5, "all").expect("有資料");
        assert_eq!(
            (response.from.as_str(), response.data_as_of.as_str()),
            ("2026-01-29", "2026-02-27")
        );
        assert_eq!(
            response
                .monthly
                .iter()
                .map(|point| point.date.as_str())
                .collect::<Vec<_>>(),
            ["2026-01-30", "2026-02-27"]
        );
        assert_eq!(response.current.median_pe, Some(13.33));
        let pe = response.pe.expect("有本益比");
        assert_eq!((pe.percentile, pe.min, pe.days), (25.0, 13.33, 4));
        assert_eq!(response.monthly[0].median_pb, Some(1.5));

        // 最新一天沒有本益比：pe、pb 為 null。
        let mut missing = days;
        missing.push(day((2026, 3, 2), None));
        let response = build_response(missing, 5, "all").expect("有資料");
        assert!(response.pe.is_none() && response.pb.is_none());
        assert!(build_response(Vec::new(), 5, "all").is_none());
    }
}
