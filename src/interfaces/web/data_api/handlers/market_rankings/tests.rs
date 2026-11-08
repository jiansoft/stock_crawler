//! `/market/dividend-yield-ranking` 與 `/market/qfii-holding-ranking` 的測試：排序白名單、
//! 參數檢查（422）與查詢結果（臨時資料庫）。

use axum::extract::Query;
use axum::http::StatusCode;

use super::{dividend_yield_ranking, qfii_holding_ranking, qfii_order_by};
use crate::interfaces::web::data_api::dto::{DividendYieldRankingParams, QfiiHoldingRankingParams};

/// §4.10 QFII 排序白名單：兩個固定分支皆為降冪加股票代號穩定排序；
/// 白名單以外的文字必須在接觸 SQL 前遭拒。
#[test]
fn qfii_sort_has_two_static_branches() {
    assert_eq!(
        qfii_order_by("percentage"),
        Ok("ORDER BY qfii_share_holding_percentage DESC, stock_symbol ASC")
    );
    assert_eq!(
        qfii_order_by("shares"),
        Ok("ORDER BY qfii_shares_held DESC, stock_symbol ASC")
    );
    for invalid in ["", "issued_share", "percentage; DROP TABLE stocks"] {
        assert!(qfii_order_by(invalid).is_err(), "{invalid:?} 應被拒絕");
    }
}

fn yield_params(
    date: Option<&str>,
    market: Option<&str>,
    industry_id: Option<i32>,
    limit: Option<u8>,
) -> Query<DividendYieldRankingParams> {
    Query(DividendYieldRankingParams {
        date: date.map(str::to_owned),
        market: market.map(str::to_owned),
        industry_id,
        limit,
    })
}

fn qfii_params(
    market: Option<&str>,
    industry_id: Option<i32>,
    sort_by: Option<&str>,
    limit: Option<u8>,
) -> Query<QfiiHoldingRankingParams> {
    Query(QfiiHoldingRankingParams {
        market: market.map(str::to_owned),
        industry_id,
        sort_by: sort_by.map(str::to_owned),
        limit,
    })
}

/// 殖利率排行：日期、市場、產業、筆數不合法時在查資料庫前回 422。
#[tokio::test]
async fn dividend_yield_ranking_rejects_invalid_params() {
    for params in [
        yield_params(Some("2026/01/01"), None, None, None),
        yield_params(None, Some("otc"), None, None),
        yield_params(None, None, Some(0), None),
        yield_params(None, None, None, Some(0)),
        yield_params(None, None, None, Some(51)),
    ] {
        assert_eq!(
            dividend_yield_ranking(params).await.status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
}

/// 外資持股排行：市場、產業、排序、筆數不合法時在查資料庫前回 422。
#[tokio::test]
async fn qfii_holding_ranking_rejects_invalid_params() {
    for params in [
        qfii_params(Some("otc"), None, None, None),
        qfii_params(None, Some(-1), None, None),
        qfii_params(None, None, Some("issued_share"), None),
        qfii_params(None, None, None, Some(0)),
    ] {
        assert_eq!(
            qfii_holding_ranking(params).await.status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
}

/// 殖利率排行回溯窗內沒有排行日期回 404；外資持股排行篩選後沒有股票回 200（空陣列）。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn rankings_query_the_database() {
    dotenvy::dotenv().ok();
    if sqlx::query("SELECT 1")
        .execute(crate::infra::database::get_connection())
        .await
        .is_err()
    {
        println!("跳過 rankings_query_the_database：無資料庫連接");
        return;
    }

    let yield_status = dividend_yield_ranking(yield_params(Some("1990-01-01"), None, None, None))
        .await
        .status();
    let qfii_status = qfii_holding_ranking(qfii_params(
        Some("twse"),
        Some(999_999),
        Some("shares"),
        Some(5),
    ))
    .await
    .status();

    assert_eq!(yield_status, StatusCode::NOT_FOUND, "1990 年沒有殖利率排行");
    assert_eq!(qfii_status, StatusCode::OK, "沒有符合的股票回空陣列");
}
