//! 版本化唯讀股票 Data API 與其線上 OpenAPI 文件。
//!
//! 此模組把 PostgreSQL 的內部 schema 隔離於 HTTP 契約之外；MCP 等內網服務
//! 只需依 `/api-docs/openapi.json` 建立 client，並可從 `/swagger-ui` 互動測試。

mod auth;
mod dto;
mod handlers;

use axum::{Router, middleware};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

/// 由 handler 註解生成的 OpenAPI 3 文件。
#[derive(OpenApi)]
#[openapi(
    paths(handlers::search_stocks, handlers::latest_quote, handlers::price_history, handlers::stock_profile, handlers::realtime_snapshot, handlers::healthz),
    components(schemas(dto::Stock, dto::DailyQuote, dto::HistoricalQuote, dto::QuoteHistoryRecord, dto::StockProfile, dto::SearchResponse, dto::LatestQuoteResponse, dto::PriceHistoryResponse, dto::RealtimeSnapshotResponse, dto::ErrorBody, dto::HealthResponse)),
    tags((name = "data-api", description = "唯讀股票資料查詢")),
    security(("bearer_auth" = [])),
    modifiers(&SecurityAddon)
)]
struct ApiDoc;

/// 將 Bearer token 宣告加入 OpenAPI components，供 Swagger UI 顯示 Authorize 按鈕。
struct SecurityAddon;
impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        openapi
            .components
            .get_or_insert_default()
            .add_security_scheme(
                "bearer_auth",
                utoipa::openapi::security::SecurityScheme::Http(
                    utoipa::openapi::security::HttpBuilder::new()
                        .scheme(utoipa::openapi::security::HttpAuthScheme::Bearer)
                        .bearer_format("API key")
                        .build(),
                ),
            );
    }
}

/// 建立 `/api/v1` 路由與不受驗證保護的 Swagger/OpenAPI 文件入口。
pub(super) fn router() -> Router {
    let protected = Router::new()
        .route(
            "/stocks/search",
            axum::routing::get(handlers::search_stocks),
        )
        .route(
            "/stocks/{symbol}/latest-quote",
            axum::routing::get(handlers::latest_quote),
        )
        .route(
            "/stocks/{symbol}/price-history",
            axum::routing::get(handlers::price_history),
        )
        .route(
            "/stocks/{symbol}/profile",
            axum::routing::get(handlers::stock_profile),
        )
        .route(
            "/stocks/{symbol}/realtime-snapshot",
            axum::routing::get(handlers::realtime_snapshot),
        )
        .layer(middleware::from_fn(auth::require_bearer_key));
    Router::new()
        .nest(
            "/api/v1",
            protected.route("/healthz", axum::routing::get(handlers::healthz)),
        )
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
}

#[cfg(test)]
mod tests {
    //! 不連資料庫的 API 基礎契約測試。
    //!
    //! 這些測試先驗證路由、驗證邊界與 OpenAPI 產物；資料庫查詢語意則由整合
    //! 測試環境驗證，避免單元測試因本機沒有 PostgreSQL 而失去可重現性。

    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    use utoipa::OpenApi;

    use super::{ApiDoc, router};

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

    /// OpenAPI 契約須列出四個資料查詢路徑與健康檢查，供 Go client codegen 使用。
    #[test]
    fn openapi_contains_all_data_api_paths() {
        let json = ApiDoc::openapi()
            .to_json()
            .expect("OpenAPI should serialize");
        for path in [
            "/api/v1/stocks/search",
            "/api/v1/stocks/{symbol}/latest-quote",
            "/api/v1/stocks/{symbol}/price-history",
            "/api/v1/stocks/{symbol}/profile",
            "/api/v1/stocks/{symbol}/realtime-snapshot",
            "/api/v1/healthz",
        ] {
            assert!(json.contains(path), "OpenAPI should contain {path}");
        }
        assert!(json.contains("bearer_auth"));
    }
}
