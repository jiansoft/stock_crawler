//! Data API 的跨層逾時預算：依路由輕重決定請求逾時與資料庫查詢預算。
//!
//! 預算由外而內遞減（Next.js BFF → Go → Rust → PostgreSQL），讓內層先逾時並回明確錯誤，
//! 而不是外層已斷線、內層仍在空轉：
//!
//! | 類型 | BFF | Go | Rust | PostgreSQL |
//! |---|---:|---:|---:|---:|
//! | 一般查詢 | 12s | 10s | 8s | 6s |
//! | 聚合分析／回測 | 35s | 32s | 30s | 25s |
//!
//! 回測上限依 pi 實測：2019 年起的滾動前推、偏移與前緣約 15～18 秒，故不採原規劃的 16 秒。

use std::time::Duration;

use axum::{extract::Request, http::StatusCode, middleware::Next, response::Response};

use super::handlers::error_response;
use crate::{
    core::logging::slow_query,
    infra::database::{self, QueryBudget},
};

/// 一般查詢的 Rust 端請求上限。
pub(crate) const GENERAL_TIMEOUT: Duration = Duration::from_secs(8);

/// 聚合分析與回測的 Rust 端請求上限。
pub(crate) const HEAVY_TIMEOUT: Duration = Duration::from_secs(30);

/// 屬於重型查詢的路徑片段：回測各端點、訊號／法人連買績效、條件選股、穿透曝險、再平衡、
/// ETF 比較與持股績效。
const HEAVY_PATHS: [&str; 7] = [
    "/backtest",
    "/performance",
    "/stocks/screen",
    "/portfolio/etf-exposure",
    "/portfolio/rebalance",
    "/portfolio/history",
    "/etfs/compare",
];

/// 依路徑判斷查詢預算。
pub(crate) fn classify(path: &str) -> QueryBudget {
    if HEAVY_PATHS.iter().any(|fragment| path.contains(fragment)) {
        QueryBudget::Heavy
    } else {
        QueryBudget::General
    }
}

/// 該預算的 Rust 端請求上限。
pub(crate) const fn request_timeout(budget: QueryBudget) -> Duration {
    match budget {
        QueryBudget::General => GENERAL_TIMEOUT,
        QueryBudget::Heavy => HEAVY_TIMEOUT,
    }
}

/// 套用請求逾時與資料庫查詢預算；逾時回 504 與標準錯誤 JSON，並記錄慢請求的端點與參數。
pub(crate) async fn enforce(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let budget = classify(&path);
    enforce_with(budget, request_timeout(budget), request, next).await
}

/// [`enforce`] 的本體；逾時長度可注入，讓測試不必真的等 8 秒。
pub(crate) async fn enforce_with(
    budget: QueryBudget,
    timeout: Duration,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or_default().to_owned();
    match tokio::time::timeout(
        timeout,
        slow_query::in_api(
            &path,
            database::with_query_budget(budget, next.run(request)),
        ),
    )
    .await
    {
        Ok(response) => response,
        Err(_) => {
            tracing::warn!(
                "Data API 請求逾時（{}s）: path={path} query={query}",
                timeout.as_secs_f32()
            );
            error_response(
                StatusCode::GATEWAY_TIMEOUT,
                "查詢逾時，請縮小查詢範圍後重試",
            )
        }
    }
}

#[cfg(test)]
mod tests {
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
        ] {
            assert_eq!(classify(path), QueryBudget::Heavy, "{path}");
        }
        for path in ["/stocks/2330/latest-quote", "/portfolio/targets", "/etfs"] {
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
}
