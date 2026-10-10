//! Data API 路由測試。
//!
//! 本檔是不連資料庫的基礎契約測試：路由、Bearer 驗證邊界與 OpenAPI 產物。
//! 資料庫查詢語意依 endpoint 分組放在子模組（`phases`、`cagr`、`movers`、`chip`、`stocks`），
//! 由整合測試環境驗證，避免單元測試因本機沒有 PostgreSQL 而失去可重現性。

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;

use super::router;

mod cagr;
mod chip;
mod movers;
mod phases;
mod stocks;

/// 健康檢查必須免驗證，讓部署系統能在未持有 API key 時偵測存活狀態。
#[tokio::test]
async fn healthz_is_public() {
    let response = router()
        .oneshot(
            Request::get("/api/v1/healthz")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("router should serve request");
    assert_eq!(response.status(), StatusCode::OK);
}

/// OpenAPI JSON 與 Swagger UI 不需驗證；升級 utoipa-swagger-ui 後確認兩個入口仍能服務。
#[tokio::test]
async fn openapi_json_and_swagger_ui_are_served() {
    let response = router()
        .oneshot(
            Request::get("/api-docs/openapi.json")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("router should serve request");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let document: serde_json::Value =
        serde_json::from_slice(&bytes).expect("OpenAPI should be JSON");
    assert!(
        document["openapi"]
            .as_str()
            .is_some_and(|v| v.starts_with("3.1"))
    );
    assert!(document["paths"]["/api/v1/market/movers"].is_object());

    let response = router()
        .oneshot(
            Request::get("/swagger-ui/")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("router should serve request");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let html = String::from_utf8_lossy(&bytes);
    assert!(
        html.contains("swagger"),
        "Swagger UI 頁面應包含 swagger 資源"
    );
}

/// 未帶 token 的受保護路徑必須在觸及資料庫前直接被拒絕。
#[tokio::test]
async fn protected_endpoint_rejects_missing_bearer_key() {
    let response = router()
        .oneshot(
            Request::get("/api/v1/stocks/search?query=2330")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("router should serve request");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// 新增的三個歷史 endpoint 也必須受 Bearer 驗證保護，未帶 token 一律 401。
#[tokio::test]
async fn phase1_endpoints_reject_missing_bearer_key() {
    for path in [
        "/api/v1/stocks/2330/monthly-revenues",
        "/api/v1/stocks/2330/financial-statements",
        "/api/v1/stocks/2330/dividends",
    ] {
        let response = router()
            .oneshot(
                Request::get(path)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("router should serve request");
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{path} 應回 401"
        );
    }
}

/// Phase 2 三個分析 endpoints 必須在 middleware 層拒絕未授權請求，
/// 確保 401 發生在任何 SQL 查詢之前。
#[tokio::test]
async fn phase2_endpoints_reject_missing_bearer_key() {
    for path in [
        "/api/v1/stocks/2330/valuation",
        "/api/v1/market/breadth",
        "/api/v1/market/dividend-yield-ranking",
    ] {
        let response = router()
            .oneshot(
                Request::get(path)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("router should serve request");
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{path} 應回 401"
        );
    }
}

/// Phase 3 選股 endpoint 必須先經 Bearer middleware，未授權請求不得執行
/// 任何每股 LATERAL SQL。
#[tokio::test]
async fn phase3_endpoint_rejects_missing_bearer_key() {
    let response = router()
        .oneshot(
            Request::get("/api/v1/stocks/screen?market=twse")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("router should serve request");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// Phase 4 三個市場輔助 endpoints 必須在 middleware 層拒絕未授權請求，
/// 確保 401 發生在任何 SQL 查詢之前。
#[tokio::test]
async fn phase4_endpoints_reject_missing_bearer_key() {
    for path in [
        "/api/v1/market/index-history",
        "/api/v1/market/dividend-calendar",
        "/api/v1/market/qfii-holding-ranking",
    ] {
        let response = router()
            .oneshot(
                Request::get(path)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("router should serve request");
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{path} 應回 401"
        );
    }
}

/// 漲跌幅／成交量排行 endpoint 也必須受 Bearer 驗證保護，未帶 token 一律 401。
#[tokio::test]
async fn movers_endpoint_rejects_missing_bearer_key() {
    let response = router()
        .oneshot(
            Request::get("/api/v1/market/movers")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("router should serve request");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// M4 兩個 endpoint 都必須在 middleware 層拒絕未授權請求。
#[tokio::test]
async fn cagr_endpoints_reject_missing_bearer_key() {
    for path in [
        "/api/v1/market/cagr-ranking",
        "/api/v1/market/cagr-ranking/2330",
    ] {
        let response = router()
            .oneshot(
                Request::get(path)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("router should serve request");
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{path} 應回 401"
        );
    }
}
