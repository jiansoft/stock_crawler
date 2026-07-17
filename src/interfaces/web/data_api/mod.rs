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
    paths(handlers::search_stocks, handlers::latest_quote, handlers::price_history, handlers::stock_profile, handlers::realtime_snapshot, handlers::monthly_revenues, handlers::financial_statements, handlers::dividend_history, handlers::healthz),
    components(schemas(dto::Stock, dto::DailyQuote, dto::HistoricalQuote, dto::QuoteHistoryRecord, dto::StockProfile, dto::SearchResponse, dto::LatestQuoteResponse, dto::PriceHistoryResponse, dto::RealtimeSnapshotResponse, dto::MonthlyRevenue, dto::MonthlyRevenueResponse, dto::FinancialStatement, dto::FinancialStatementHistoryResponse, dto::Dividend, dto::DividendHistoryResponse, dto::ErrorBody, dto::HealthResponse)),
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

    /// OpenAPI 契約須列出所有資料查詢路徑與健康檢查，供 Go client codegen 使用。
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
            "/api/v1/stocks/{symbol}/monthly-revenues",
            "/api/v1/stocks/{symbol}/financial-statements",
            "/api/v1/stocks/{symbol}/dividends",
            "/api/v1/healthz",
        ] {
            assert!(json.contains(path), "OpenAPI should contain {path}");
        }
        assert!(json.contains("bearer_auth"));
    }

    /// Phase 1 三個歷史 endpoint 的 response schema 必須固定欄位名稱與
    /// null 語意（P0-5）：抽查 envelope 欄位與可為 null 的關鍵欄位。
    #[test]
    fn openapi_phase1_schemas_pin_field_names() {
        let json = ApiDoc::openapi()
            .to_json()
            .expect("OpenAPI should serialize");
        for field in [
            // 月營收 envelope 與欄位。
            "MonthlyRevenueResponse",
            "\"revenues\"",
            "\"month_over_month_percent\"",
            // 財報 envelope 與 §4.2 改名後的欄位。
            "FinancialStatementHistoryResponse",
            "\"statements\"",
            "\"gross_profit_margin\"",
            "\"profit_before_tax_per_share\"",
            // 股利 envelope 與 §4.3 改名後的欄位。
            "DividendHistoryResponse",
            "\"dividends\"",
            "\"paid_year\"",
            "\"dividend_year\"",
            "\"ex_dividend_date\"",
            "\"total_dividend\"",
            // 三個 envelope 共通的資料日期欄位。
            "\"data_as_of\"",
        ] {
            assert!(json.contains(field), "OpenAPI should contain {field}");
        }
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

    /// Phase 1 endpoints 的資料庫語意整合測試（§3.2、§3.3）。
    ///
    /// 覆蓋三種語意：未知代號 → 404；已知代號但指定範圍無資料 → 200 空
    /// 陣列且 `data_as_of` 為 null；參數不合法 → 422。無資料庫時跳過。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn phase1_endpoints_db_semantics() {
        dotenvy::dotenv().ok();
        if sqlx::query("SELECT 1")
            .execute(crate::infra::database::get_connection())
            .await
            .is_err()
        {
            println!("跳過 phase1_endpoints_db_semantics：無資料庫連接");
            return;
        }
        // Auth middleware 讀環境變數 DATA_API_KEY；測試環境沒設定時自行
        // 補一組（--test-threads=1 下無資料競爭疑慮）。
        let key = std::env::var("DATA_API_KEY").unwrap_or_else(|_| {
            let generated = "phase1-integration-test-key".to_owned();
            unsafe { std::env::set_var("DATA_API_KEY", &generated) };
            generated
        });
        // 以真實 router 發出帶 token 的請求並解析 JSON body。
        let get = |path: &str| {
            let path = path.to_owned();
            let key = key.clone();
            async move {
                let response = router()
                    .oneshot(
                        Request::get(&path)
                            .header("Authorization", format!("Bearer {key}"))
                            .body(Body::empty())
                            .expect("request should build"),
                    )
                    .await
                    .expect("router should serve request");
                let status = response.status();
                let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .expect("body should be readable");
                let json: serde_json::Value =
                    serde_json::from_slice(&bytes).expect("body should be JSON");
                (status, json)
            }
        };

        // 語意一：未知代號在三個 endpoint 都必須是 404。
        for path in [
            "/api/v1/stocks/NO_SUCH_SYMBOL/monthly-revenues",
            "/api/v1/stocks/NO_SUCH_SYMBOL/financial-statements",
            "/api/v1/stocks/NO_SUCH_SYMBOL/dividends",
        ] {
            let (status, _) = get(path).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path} 未知代號應回 404");
        }

        // 語意二：已知代號 + 必然無資料的範圍 → 200 空陣列、data_as_of null。
        // 從 stocks 任取一檔存在的股票，避免測試依賴特定代號。
        let symbol: Option<(String,)> = sqlx::query_as("SELECT stock_symbol FROM stocks LIMIT 1")
            .fetch_optional(crate::infra::database::get_connection())
            .await
            .expect("stocks query should work");
        let Some((symbol,)) = symbol else {
            println!("跳過語意二：stocks 表無資料");
            return;
        };
        let (status, json) = get(&format!(
            "/api/v1/stocks/{symbol}/monthly-revenues?from=1990-01&to=1990-02"
        ))
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["revenues"], serde_json::json!([]), "空範圍應回空陣列");
        assert_eq!(json["data_as_of"], serde_json::Value::Null);
        // 股利的「空範圍」不能假設任意年份沒資料（台股歷史夠久，1990 也可能
        // 有配息），改由資料庫找出該股票最早的股利所屬年度，取其前一年驗證。
        let min_year: Option<(Option<i32>,)> =
            sqlx::query_as("SELECT MIN(year_of_dividend) FROM dividend WHERE security_code = $1")
                .bind(&symbol)
                .fetch_optional(crate::infra::database::get_connection())
                .await
                .expect("dividend min-year query should work");
        let empty_year = match min_year.and_then(|(value,)| value) {
            // 最早年度的前一年必然無資料；早於等於 1990 的極端情形跳過此斷言。
            Some(min) if min > 1990 => Some(min - 1),
            Some(_) => None,
            // 這檔股票完全沒有股利資料，任何合法年份都應回空陣列。
            None => Some(1990),
        };
        if let Some(year) = empty_year {
            let (status, json) = get(&format!(
                "/api/v1/stocks/{symbol}/dividends?from_year={year}&to_year={year}"
            ))
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(
                json["dividends"],
                serde_json::json!([]),
                "無資料年份應回空陣列"
            );
            assert_eq!(json["data_as_of"], serde_json::Value::Null);
        }
        let (status, json) = get(&format!(
            "/api/v1/stocks/{symbol}/financial-statements?limit=1"
        ))
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(json["statements"].is_array(), "statements 必須是陣列");

        // 語意三：參數不合法 → 422（在存在性檢查之前就擋下）。
        for path in [
            format!("/api/v1/stocks/{symbol}/monthly-revenues?from=2026-13"),
            format!("/api/v1/stocks/{symbol}/monthly-revenues?from=2026-06&to=2026-01"),
            format!("/api/v1/stocks/{symbol}/monthly-revenues?limit=121"),
            format!("/api/v1/stocks/{symbol}/financial-statements?period_type=monthly"),
            format!("/api/v1/stocks/{symbol}/financial-statements?limit=0"),
            format!("/api/v1/stocks/{symbol}/dividends?from_year=1889"),
            format!("/api/v1/stocks/{symbol}/dividends?from_year=2024&to_year=2020"),
            format!("/api/v1/stocks/{symbol}/dividends?limit=81"),
        ] {
            let (status, _) = get(&path).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{path} 應回 422");
        }
    }
}
