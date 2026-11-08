//! Data API 的跨層逾時預算：依路由輕重決定請求逾時與資料庫查詢預算的測試。

use axum::{Router, body::Body, middleware, routing::get};
use tower::ServiceExt;

use super::*;

#[test]
fn classifies_heavy_paths() {
    for path in [
        "/api/v1/stocks/screen/backtest/walk-forward",
        "/stocks/screen",
        "/market/signals/performance",
        "/portfolio/etf-exposure",
        "/portfolio/rebalance",
        "/etfs/compare",
        "/api/v1/portfolio/dividend-fill",
        "/api/v1/portfolio/dividend-forecast",
        "/api/v1/portfolio/dividend-forecast/accuracy",
    ] {
        assert_eq!(classify(path), QueryBudget::Heavy, "{path}");
    }
    for path in [
        "/stocks/2330/latest-quote",
        "/portfolio/targets",
        "/etfs",
        "/stocks/2330/dividend-fill",
    ] {
        assert_eq!(classify(path), QueryBudget::General, "{path}");
    }
    assert_eq!(request_timeout(QueryBudget::General), GENERAL_TIMEOUT);
    assert_eq!(request_timeout(QueryBudget::Heavy), HEAVY_TIMEOUT);
    // 內層必須比外層先逾時：Rust 請求上限大於資料庫上限。
    for budget in [QueryBudget::General, QueryBudget::Heavy] {
        assert!(request_timeout(budget) > budget.statement_timeout());
    }
}

fn app(timeout: Duration) -> Router {
    Router::new()
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                "done"
            }),
        )
        .route(
            "/budget",
            get(|| async { format!("{:?}", database::current_query_budget()) }),
        )
        .layer(middleware::from_fn(move |request, next| {
            enforce_with(QueryBudget::Heavy, timeout, request, next)
        }))
}

#[tokio::test]
async fn timeout_returns_504_json() {
    let response = app(Duration::from_millis(20))
        .oneshot(Request::get("/slow").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["error"].as_str().unwrap().contains("逾時"));
}

#[tokio::test]
async fn handler_runs_inside_query_budget() {
    let response = app(Duration::from_secs(5))
        .oneshot(Request::get("/budget").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&body[..], b"Some(Heavy)");
    assert_eq!(database::current_query_budget(), None);
}

#[tokio::test]
async fn enforce_classifies_by_path() {
    let app = Router::new()
        .route(
            "/stocks/screen/backtest",
            get(|| async { format!("{:?}", database::current_query_budget()) }),
        )
        .route(
            "/stocks/search",
            get(|| async { format!("{:?}", database::current_query_budget()) }),
        )
        .layer(middleware::from_fn(enforce));
    for (path, expected) in [
        ("/stocks/screen/backtest", "Some(Heavy)"),
        ("/stocks/search", "Some(General)"),
    ] {
        let response = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], expected.as_bytes(), "{path}");
    }
}
