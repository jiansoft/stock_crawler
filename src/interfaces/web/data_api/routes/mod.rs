//! Data API 的路由註冊。
//!
//! `/api/v1` 下除健康檢查外一律套用 Bearer middleware；Swagger UI 與
//! OpenAPI JSON 不受驗證保護，方便內網服務直接產生 client。

use axum::{Router, middleware};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use super::openapi::ApiDoc;
use super::{auth, handlers};

/// 建立 `/api/v1` 路由與不受驗證保護的 Swagger/OpenAPI 文件入口。
pub(crate) fn router() -> Router {
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
        .route(
            "/stocks/{symbol}/monthly-revenues",
            axum::routing::get(handlers::monthly_revenues),
        )
        .route(
            "/stocks/{symbol}/financial-statements",
            axum::routing::get(handlers::financial_statements),
        )
        .route(
            "/stocks/{symbol}/dividends",
            axum::routing::get(handlers::dividend_history),
        )
        .route(
            "/stocks/{symbol}/chip",
            axum::routing::get(handlers::stock_chip),
        )
        .route(
            "/stocks/{symbol}/valuation",
            axum::routing::get(handlers::stock_valuation),
        )
        .route(
            "/market/breadth",
            axum::routing::get(handlers::market_breadth),
        )
        .route(
            "/market/dividend-yield-ranking",
            axum::routing::get(handlers::dividend_yield_ranking),
        )
        .route(
            "/stocks/screen",
            axum::routing::get(handlers::screen_stocks),
        )
        .route(
            "/market/index-history",
            axum::routing::get(handlers::market_index_history),
        )
        .route(
            "/market/dividend-calendar",
            axum::routing::get(handlers::dividend_calendar),
        )
        .route(
            "/market/qfii-holding-ranking",
            axum::routing::get(handlers::qfii_holding_ranking),
        )
        .route(
            "/market/movers",
            axum::routing::get(handlers::market_movers),
        )
        .route(
            "/market/cagr-ranking",
            axum::routing::get(handlers::cagr_ranking),
        )
        .route(
            "/market/cagr-ranking/{stock_symbol}",
            axum::routing::get(handlers::cagr_by_symbol),
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
mod tests;
