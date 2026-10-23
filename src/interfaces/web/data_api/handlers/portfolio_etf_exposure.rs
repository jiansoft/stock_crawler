//! `/portfolio/etf-exposure` handler：持股穿透曝險。
//!
//! 持有 ETF 的市值依前十大持股權重拆到成分股，與直接持股合併（規則在
//! [`crate::domain::portfolio::exposure`]）。屬個人持股資料。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use super::{error_response, repository_error, round2};
use crate::domain::portfolio::exposure::{ExposureReport, build_report};
use crate::infra::database::repository::portfolio::PgPortfolioRepository;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, ExposureEtfItem, ExposureItem, ExposureSourceItem, PortfolioEtfExposureParams,
    PortfolioEtfExposureResponse,
};

/// 預設回傳幾檔成分股。
const DEFAULT_LIMIT: u16 = 20;

/// 最多回傳幾檔成分股。
const MAX_LIMIT: u16 = 200;

/// 查詢持股穿透曝險：直接持股加上經由 ETF 前十大持股間接持有的金額。
///
/// # Errors
///
/// `member_id` 為負或 `limit` 超出 1–200 回 422；驗證失敗回 401；資料庫查詢失敗回不含 SQL 細節的
/// 500。沒有持股時回 200 與空清單。
#[utoipa::path(get, path = "/api/v1/portfolio/etf-exposure", tag = "data-api", params(PortfolioEtfExposureParams), responses((status = 200, body = PortfolioEtfExposureResponse), (status = 401, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn portfolio_etf_exposure(
    Query(params): Query<PortfolioEtfExposureParams>,
) -> Response {
    let member_id = params.member_id.unwrap_or(0);
    if member_id < 0 {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "member_id 不可為負數");
    }
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 200");
    }
    match build_report(&PgPortfolioRepository::new(), member_id).await {
        Ok(report) => Json(response(report, usize::from(limit))).into_response(),
        Err(error) => repository_error(error),
    }
}

/// 金額四捨五入到元。
fn round0(value: f64) -> f64 {
    value.round()
}

/// 組出回應：成分股只回傳前 `limit` 檔，權重以有收盤價的總市值為分母。
fn response(report: ExposureReport, limit: usize) -> PortfolioEtfExposureResponse {
    let total = report.total_value;
    let total_items = report.exposures.len();
    PortfolioEtfExposureResponse {
        total_value: round0(total),
        etf_value: round0(report.etf_value),
        uncovered_value: round0(report.uncovered_value),
        total_items,
        etfs: report
            .etfs
            .into_iter()
            .map(|etf| ExposureEtfItem {
                stock_symbol: etf.stock_symbol,
                name: etf.name,
                market_value: round0(etf.market_value),
                holdings_as_of: etf.holdings_as_of.map(|date| date.to_string()),
                covered_weight: round2(etf.covered_weight),
            })
            .collect(),
        items: report
            .exposures
            .into_iter()
            .take(limit)
            .map(|exposure| ExposureItem {
                etf_value: round0(exposure.etf_value()),
                total_value: round0(exposure.total_value()),
                weight: if total > 0.0 {
                    round2(exposure.total_value() / total * 100.0)
                } else {
                    0.0
                },
                direct_value: round0(exposure.direct_value),
                sources: exposure
                    .sources
                    .into_iter()
                    .map(|source| ExposureSourceItem {
                        etf_symbol: source.etf_symbol,
                        value: round0(source.value),
                    })
                    .collect(),
                symbol: exposure.symbol,
                name: exposure.name,
            })
            .collect(),
        unpriced: report.unpriced,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::portfolio::exposure::{EtfConstituent, ExposurePosition, look_through};

    #[test]
    fn response_rounds_money_and_limits_items() {
        let report = look_through(
            &[
                ExposurePosition {
                    stock_symbol: "0050".to_owned(),
                    name: "元大台灣50".to_owned(),
                    market_value: Some(1_000.4),
                    is_etf: true,
                },
                ExposurePosition {
                    stock_symbol: "2317".to_owned(),
                    name: "鴻海".to_owned(),
                    market_value: Some(100.0),
                    is_etf: false,
                },
            ],
            &[EtfConstituent {
                etf_symbol: "0050".to_owned(),
                as_of: chrono::NaiveDate::from_ymd_opt(2026, 9, 30).expect("日期合法"),
                holding_symbol: "2330".to_owned(),
                holding_name: "台積電".to_owned(),
                weight: 56.456,
            }],
        );
        let response = response(report, 1);
        assert_eq!(response.total_value, 1_100.0);
        assert_eq!(response.total_items, 2);
        assert_eq!(response.items.len(), 1);
        let tsmc = &response.items[0];
        assert_eq!((tsmc.symbol.as_str(), tsmc.total_value), ("2330", 565.0));
        assert_eq!(tsmc.weight, 51.33);
        assert_eq!(tsmc.sources[0].etf_symbol, "0050");
        assert_eq!(response.etfs[0].covered_weight, 56.46);
        assert_eq!(
            response.etfs[0].holdings_as_of.as_deref(),
            Some("2026-09-30")
        );
    }

    #[test]
    fn an_empty_portfolio_has_zero_weights() {
        let response = response(ExposureReport::default(), 20);
        assert_eq!(response.total_value, 0.0);
        assert!(response.items.is_empty() && response.etfs.is_empty());
    }
}
