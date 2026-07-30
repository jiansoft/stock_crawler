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
    paths(handlers::search_stocks, handlers::latest_quote, handlers::price_history, handlers::stock_profile, handlers::realtime_snapshot, handlers::monthly_revenues, handlers::financial_statements, handlers::dividend_history, handlers::stock_valuation, handlers::market_breadth, handlers::dividend_yield_ranking, handlers::screen_stocks, handlers::market_index_history, handlers::dividend_calendar, handlers::qfii_holding_ranking, handlers::market_movers, handlers::healthz),
    components(schemas(dto::Stock, dto::DailyQuote, dto::HistoricalQuote, dto::QuoteHistoryRecord, dto::StockProfile, dto::SearchResponse, dto::LatestQuoteResponse, dto::PriceHistoryResponse, dto::RealtimeSnapshotResponse, dto::MonthlyRevenue, dto::MonthlyRevenueResponse, dto::FinancialStatement, dto::FinancialStatementHistoryResponse, dto::Dividend, dto::DividendHistoryResponse, dto::StockValuation, dto::StockValuationResponse, dto::MarketBreadth, dto::MarketBreadthResponse, dto::DividendYieldRank, dto::DividendYieldRankingResponse, dto::ScreenedStock, dto::StockScreeningResponse, dto::MarketIndexPoint, dto::MarketIndexHistoryResponse, dto::DividendCalendarEvent, dto::DividendCalendarResponse, dto::QfiiHolding, dto::QfiiHoldingRankingResponse, dto::MarketMover, dto::MarketMoversResponse, dto::ErrorBody, dto::HealthResponse)),
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

    /// 取得指定 GET operation；path 缺漏時立即顯示精確路徑。
    fn get_operation<'a>(document: &'a serde_json::Value, path: &str) -> &'a serde_json::Value {
        document["paths"][path]["get"]
            .as_object()
            .map(|_| &document["paths"][path]["get"])
            .unwrap_or_else(|| panic!("OpenAPI 缺少 GET {path}"))
    }

    /// 驗證 operation 的 status code 指向指定 component schema。
    fn assert_response(operation: &serde_json::Value, status: &str, schema: &str) {
        let reference =
            operation["responses"][status]["content"]["application/json"]["schema"]["$ref"]
                .as_str()
                .unwrap_or_else(|| panic!("response {status} 缺少 JSON schema ref"));
        assert_eq!(reference, format!("#/components/schemas/{schema}"));
    }

    /// 取得 query parameter schema，避免以全文 contains 造成跨 path 假陽性。
    fn query_schema<'a>(operation: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
        operation["parameters"]
            .as_array()
            .expect("parameters 應為陣列")
            .iter()
            .find(|parameter| parameter["name"] == name && parameter["in"] == "query")
            .map(|parameter| &parameter["schema"])
            .unwrap_or_else(|| panic!("缺少 query parameter {name}"))
    }

    /// 解析 utoipa 可能 inline、`$ref` 或以 `allOf` 包裝的 enum schema。
    fn enum_values(document: &serde_json::Value, schema: &serde_json::Value) -> serde_json::Value {
        if schema["enum"].is_array() {
            return schema["enum"].clone();
        }
        if let Some(reference) = schema["$ref"].as_str() {
            let resolved = document
                .pointer(reference.trim_start_matches('#'))
                .expect("enum $ref 應指向有效 component");
            return enum_values(document, resolved);
        }
        if let Some(all_of) = schema["allOf"].as_array() {
            for nested in all_of {
                let values = enum_values(document, nested);
                if values.is_array() {
                    return values;
                }
            }
        }
        if let Some(any_of) = schema["anyOf"].as_array() {
            for nested in any_of {
                let values = enum_values(document, nested);
                if values.is_array() {
                    return values;
                }
            }
        }
        if let Some(one_of) = schema["oneOf"].as_array() {
            for nested in one_of {
                let values = enum_values(document, nested);
                if values.is_array() {
                    return values;
                }
            }
        }
        panic!("無法解析 enum schema: {schema}")
    }

    /// 驗證成功與標準錯誤 responses；`has_not_found` 控制是否要求 404。
    fn assert_endpoint_responses(
        operation: &serde_json::Value,
        success_schema: &str,
        has_not_found: bool,
    ) {
        assert_response(operation, "200", success_schema);
        for status in ["401", "422", "500"] {
            assert_response(operation, status, "ErrorBody");
        }
        if has_not_found {
            assert_response(operation, "404", "ErrorBody");
        }
    }

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
            "/api/v1/stocks/{symbol}/valuation",
            "/api/v1/market/breadth",
            "/api/v1/market/dividend-yield-ranking",
            "/api/v1/stocks/screen",
            "/api/v1/market/index-history",
            "/api/v1/market/dividend-calendar",
            "/api/v1/market/qfii-holding-ranking",
            "/api/v1/market/movers",
            "/api/v1/healthz",
        ] {
            assert!(json.contains(path), "OpenAPI should contain {path}");
        }
        assert!(json.contains("bearer_auth"));
    }

    /// Phase 1 三條 path 分別固定 responses、query constraints、陣列 item 與
    /// nullable envelope；所有斷言皆從該 path/component 定位，不做全文搜尋。
    #[test]
    fn openapi_phase1_schemas_pin_field_names() {
        let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
        let cases = [
            (
                "/api/v1/stocks/{symbol}/monthly-revenues",
                "MonthlyRevenueResponse",
                "limit",
                24,
                1,
                120,
                "revenues",
                "MonthlyRevenue",
            ),
            (
                "/api/v1/stocks/{symbol}/financial-statements",
                "FinancialStatementHistoryResponse",
                "limit",
                12,
                1,
                40,
                "statements",
                "FinancialStatement",
            ),
            (
                "/api/v1/stocks/{symbol}/dividends",
                "DividendHistoryResponse",
                "limit",
                20,
                1,
                80,
                "dividends",
                "Dividend",
            ),
        ];
        for (path, response, limit_name, default, minimum, maximum, list, item) in cases {
            let operation = get_operation(&document, path);
            assert_endpoint_responses(operation, response, true);
            let limit = query_schema(operation, limit_name);
            assert_eq!(limit["default"], default);
            assert_eq!(limit["minimum"], minimum);
            assert_eq!(limit["maximum"], maximum);
            let properties = &document["components"]["schemas"][response]["properties"];
            assert_eq!(properties[list]["type"], "array");
            assert_eq!(
                properties[list]["items"]["$ref"],
                format!("#/components/schemas/{item}")
            );
            assert!(properties["data_as_of"].to_string().contains("null"));
        }
        let period = query_schema(
            get_operation(&document, "/api/v1/stocks/{symbol}/financial-statements"),
            "period_type",
        );
        assert_eq!(period["default"], "quarterly");
        assert_eq!(
            enum_values(&document, period),
            serde_json::json!(["quarterly", "annual", "all"])
        );
    }

    /// Phase 2 每條 path 精確驗證 responses 與 query enum/range/default。
    #[test]
    fn openapi_phase2_schemas_pin_field_names() {
        let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
        let valuation = get_operation(&document, "/api/v1/stocks/{symbol}/valuation");
        assert_endpoint_responses(valuation, "StockValuationResponse", true);
        let breadth = get_operation(&document, "/api/v1/market/breadth");
        assert_endpoint_responses(breadth, "MarketBreadthResponse", true);
        assert_eq!(query_schema(breadth, "days")["default"], 1);
        assert_eq!(query_schema(breadth, "days")["minimum"], 1);
        assert_eq!(query_schema(breadth, "days")["maximum"], 60);
        assert_eq!(
            enum_values(&document, query_schema(breadth, "market")),
            serde_json::json!(["all", "twse", "tpex"])
        );
        let history =
            &document["components"]["schemas"]["MarketBreadthResponse"]["properties"]["history"];
        assert_eq!(history["type"], "array");
        assert_eq!(
            history["items"]["$ref"],
            "#/components/schemas/MarketBreadth"
        );
        let ranking = get_operation(&document, "/api/v1/market/dividend-yield-ranking");
        assert_endpoint_responses(ranking, "DividendYieldRankingResponse", true);
        assert_eq!(query_schema(ranking, "limit")["default"], 20);
        assert_eq!(query_schema(ranking, "industry_id")["minimum"], 1);
    }

    /// Phase 3 path 精確驗證 responses、白名單 enum、數值範圍與陣列 item。
    #[test]
    fn openapi_phase3_schema_pins_filters_and_source_dates() {
        let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
        let operation = get_operation(&document, "/api/v1/stocks/screen");
        assert_endpoint_responses(operation, "StockScreeningResponse", false);
        assert_eq!(query_schema(operation, "limit")["default"], 20);
        assert_eq!(
            enum_values(&document, query_schema(operation, "sort_order")),
            serde_json::json!(["asc", "desc"])
        );
        assert_eq!(
            enum_values(&document, query_schema(operation, "valuation_band")),
            serde_json::json!([
                "undervalued",
                "fair_valued",
                "overvalued",
                "highly_overvalued"
            ])
        );
        assert_eq!(
            query_schema(operation, "min_dividend_yield_percent")["minimum"],
            0
        );
        assert_eq!(
            query_schema(operation, "min_dividend_yield_percent")["maximum"],
            1000
        );
        let stocks =
            &document["components"]["schemas"]["StockScreeningResponse"]["properties"]["stocks"];
        assert_eq!(stocks["type"], "array");
        assert_eq!(
            stocks["items"]["$ref"],
            "#/components/schemas/ScreenedStock"
        );
    }

    /// Phase 4 三條 path 精確驗證 responses、query enum/range/default 與
    /// 陣列 item；三個市場輔助 endpoint 都沒有 404 語意（查無資料回 200
    /// 空陣列），因此 `has_not_found = false`。
    #[test]
    fn openapi_phase4_schemas_pin_field_names() {
        let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");

        // §4.8 指數歷史：limit 1–365 預設 30；points 為 MarketIndexPoint
        // 陣列；data_as_of 可為 null（空清單語意）。
        let index_history = get_operation(&document, "/api/v1/market/index-history");
        assert_endpoint_responses(index_history, "MarketIndexHistoryResponse", false);
        let limit = query_schema(index_history, "limit");
        assert_eq!(limit["default"], 30);
        assert_eq!(limit["minimum"], 1);
        assert_eq!(limit["maximum"], 365);
        let properties =
            &document["components"]["schemas"]["MarketIndexHistoryResponse"]["properties"];
        assert_eq!(properties["points"]["type"], "array");
        assert_eq!(
            properties["points"]["items"]["$ref"],
            "#/components/schemas/MarketIndexPoint"
        );
        assert!(properties["data_as_of"].to_string().contains("null"));

        // §4.9 行事曆：event_type 五值 enum 預設 all；limit 1–200 預設 50；
        // events 為 DividendCalendarEvent 陣列。
        let calendar = get_operation(&document, "/api/v1/market/dividend-calendar");
        assert_endpoint_responses(calendar, "DividendCalendarResponse", false);
        let event_type = query_schema(calendar, "event_type");
        assert_eq!(event_type["default"], "all");
        assert_eq!(
            enum_values(&document, event_type),
            serde_json::json!([
                "ex_dividend",
                "ex_rights",
                "cash_payable",
                "stock_payable",
                "all"
            ])
        );
        let limit = query_schema(calendar, "limit");
        assert_eq!(limit["default"], 50);
        assert_eq!(limit["minimum"], 1);
        assert_eq!(limit["maximum"], 200);
        let properties =
            &document["components"]["schemas"]["DividendCalendarResponse"]["properties"];
        assert_eq!(properties["events"]["type"], "array");
        assert_eq!(
            properties["events"]["items"]["$ref"],
            "#/components/schemas/DividendCalendarEvent"
        );
        assert!(properties["data_as_of"].to_string().contains("null"));

        // §4.10 QFII 排行：market 三值 enum、sort_by 兩值 enum 預設
        // percentage、industry_id 正整數、limit 1–50 預設 20；stocks 為
        // QfiiHolding 陣列。
        let qfii = get_operation(&document, "/api/v1/market/qfii-holding-ranking");
        assert_endpoint_responses(qfii, "QfiiHoldingRankingResponse", false);
        assert_eq!(
            enum_values(&document, query_schema(qfii, "market")),
            serde_json::json!(["all", "twse", "tpex"])
        );
        let sort_by = query_schema(qfii, "sort_by");
        assert_eq!(sort_by["default"], "percentage");
        assert_eq!(
            enum_values(&document, sort_by),
            serde_json::json!(["percentage", "shares"])
        );
        assert_eq!(query_schema(qfii, "industry_id")["minimum"], 1);
        let limit = query_schema(qfii, "limit");
        assert_eq!(limit["default"], 20);
        assert_eq!(limit["minimum"], 1);
        assert_eq!(limit["maximum"], 50);
        let properties =
            &document["components"]["schemas"]["QfiiHoldingRankingResponse"]["properties"];
        assert_eq!(properties["stocks"]["type"], "array");
        assert_eq!(
            properties["stocks"]["items"]["$ref"],
            "#/components/schemas/QfiiHolding"
        );
        assert!(properties["data_as_of"].to_string().contains("null"));
    }

    /// 漲跌幅／成交量排行的 OpenAPI 契約（movers 計畫 §4）。
    ///
    /// 除了一般的參數與 response 形狀，這裡刻意固定兩件容易被後人「順手統一」
    /// 而破壞語意的事：
    /// 1. `rank_by` 只有三個值，**沒有** `top_trade_value`（即時快照沒有成交
    ///    金額，提供它會讓同一參數在兩個時段語意不同）。
    /// 2. `is_realtime` 是布林且**不可為 null**——本 endpoint 是唯一可能回
    ///    `true` 的分析型 endpoint，其餘工具固定 `false`。
    #[test]
    fn openapi_movers_schema_pins_sources_and_units() {
        let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");

        let movers = get_operation(&document, "/api/v1/market/movers");
        // 這個 endpoint 在完全沒有資料時回 404（部署異常等級），因此帶
        // has_not_found = true。
        assert_endpoint_responses(movers, "MarketMoversResponse", true);

        let rank_by = query_schema(movers, "rank_by");
        assert_eq!(rank_by["default"], "top_gainers");
        assert_eq!(
            enum_values(&document, rank_by),
            serde_json::json!(["top_gainers", "top_losers", "top_volume"])
        );
        assert_eq!(
            enum_values(&document, query_schema(movers, "market")),
            serde_json::json!(["all", "twse", "tpex"])
        );
        let limit = query_schema(movers, "limit");
        assert_eq!(limit["default"], 20);
        assert_eq!(limit["minimum"], 1);
        assert_eq!(limit["maximum"], 50);

        let properties = &document["components"]["schemas"]["MarketMoversResponse"]["properties"];
        assert_eq!(properties["movers"]["type"], "array");
        assert_eq!(
            properties["movers"]["items"]["$ref"],
            "#/components/schemas/MarketMover"
        );
        // data_as_of 一定有值（即時為今日、收盤為該交易日），不可為 null。
        assert!(!properties["data_as_of"].to_string().contains("null"));
        assert_eq!(properties["is_realtime"]["type"], "boolean");
        // snapshot_updated_at 只有即時來源有值，必須是 nullable。
        assert!(
            properties["snapshot_updated_at"]
                .to_string()
                .contains("null")
        );

        // 兩種成交量單位是分開的欄位，且都可為 null——這是「不做張／股換算」
        // 這個決策在契約上的體現，不可被合併成單一欄位。
        let mover = &document["components"]["schemas"]["MarketMover"]["properties"];
        for field in [
            "volume_lots",
            "volume_shares",
            "trade_value",
            "transaction",
            "last_close",
            "source_site",
        ] {
            assert!(
                mover[field].to_string().contains("null"),
                "{field} 應為 nullable"
            );
        }
    }

    /// 排行 endpoint 也必須受 Bearer 驗證保護，未帶 token 一律 401。
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
        // quarterly 不得混入年度 alias；all 則將 DB 空字串穩定輸出成 A，且
        // year/period 必須維持由新到舊。
        let (_, quarterly) = get(&format!(
            "/api/v1/stocks/{symbol}/financial-statements?period_type=quarterly&limit=40"
        ))
        .await;
        assert!(
            quarterly["statements"]
                .as_array()
                .expect("statements array")
                .iter()
                .all(|row| matches!(row["quarter"].as_str(), Some("Q1" | "Q2" | "Q3" | "Q4")))
        );
        let (_, all_periods) = get(&format!(
            "/api/v1/stocks/{symbol}/financial-statements?period_type=all&limit=40"
        ))
        .await;
        let statements = all_periods["statements"]
            .as_array()
            .expect("statements array");
        let period_rank = |quarter: &str| match quarter {
            "A" => 7,
            "H2" => 6,
            "H1" => 5,
            "Q4" => 4,
            "Q3" => 3,
            "Q2" => 2,
            "Q1" => 1,
            _ => 0,
        };
        for pair in statements.windows(2) {
            let left = (
                pair[0]["year"].as_i64().unwrap(),
                period_rank(pair[0]["quarter"].as_str().unwrap()),
            );
            let right = (
                pair[1]["year"].as_i64().unwrap(),
                period_rank(pair[1]["quarter"].as_str().unwrap()),
            );
            assert!(left >= right, "財報必須依年度與期間新到舊");
        }
        let empty_symbol: Option<String> = sqlx::query_scalar("SELECT s.stock_symbol FROM stocks s WHERE NOT EXISTS (SELECT 1 FROM financial_statement f WHERE f.security_code = s.stock_symbol) LIMIT 1")
            .fetch_optional(crate::infra::database::get_connection()).await.expect("empty financial symbol query");
        if let Some(empty_symbol) = empty_symbol {
            let (status, json) = get(&format!(
                "/api/v1/stocks/{empty_symbol}/financial-statements"
            ))
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(json["statements"], serde_json::json!([]));
            assert!(json["data_as_of"].is_null());
        }

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

    /// Phase 2 三個 authenticated endpoints 的真實 SQL 欄位、型別、JOIN 與
    /// 404／空資料／排序語意整合測試；不建立 fixture，避免污染共享資料庫。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn phase2_endpoints_db_semantics() {
        dotenvy::dotenv().ok();
        let pool = crate::infra::database::get_connection();
        if sqlx::query("SELECT 1").execute(pool).await.is_err() {
            println!("跳過 phase2_endpoints_db_semantics：無資料庫連接");
            return;
        }
        let key = std::env::var("DATA_API_KEY").unwrap_or_else(|_| {
            let generated = "phase2-integration-test-key".to_owned();
            unsafe { std::env::set_var("DATA_API_KEY", &generated) };
            generated
        });
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
                    .expect("body readable");
                let json: serde_json::Value =
                    serde_json::from_slice(&bytes).expect("body should be JSON");
                (status, json)
            }
        };

        let (status, _) = get("/api/v1/stocks/NO_SUCH_SYMBOL/valuation").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let symbol: Option<String> =
            sqlx::query_scalar("SELECT security_code FROM estimate ORDER BY date DESC LIMIT 1")
                .fetch_optional(pool)
                .await
                .expect("estimate symbol query");
        if let Some(symbol) = symbol {
            let (status, json) = get(&format!(
                "/api/v1/stocks/{symbol}/valuation?date=1900-01-01"
            ))
            .await;
            assert_eq!(status, StatusCode::OK);
            assert!(json["valuation"].is_null());
            let (status, json) = get(&format!("/api/v1/stocks/{symbol}/valuation")).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(json["valuation"]["stock_symbol"], symbol);
            assert_eq!(json["data_as_of"], json["valuation"]["date"]);
        }

        for market in ["all", "twse", "tpex"] {
            let (status, json) =
                get(&format!("/api/v1/market/breadth?market={market}&days=3")).await;
            if status == StatusCode::NOT_FOUND {
                continue;
            }
            assert_eq!(status, StatusCode::OK);
            let history = json["history"].as_array().expect("history array");
            assert!((1..=3).contains(&history.len()));
            assert_eq!(json["breadth"], history[0]);
            assert!(history.iter().all(|row| row["market"] == market));
        }
        let (status, _) = get("/api/v1/market/breadth?date=1900-01-01").await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        for market in ["all", "twse", "tpex"] {
            let (status, json) = get(&format!(
                "/api/v1/market/dividend-yield-ranking?market={market}&limit=50"
            ))
            .await;
            if status == StatusCode::NOT_FOUND {
                continue;
            }
            assert_eq!(status, StatusCode::OK);
            let stocks = json["stocks"].as_array().expect("stocks array");
            for pair in stocks.windows(2) {
                let left = pair[0]["dividend_yield_percent"].as_f64().unwrap();
                let right = pair[1]["dividend_yield_percent"].as_f64().unwrap();
                assert!(left >= right);
                if left == right {
                    assert!(pair[0]["stock_symbol"].as_str() <= pair[1]["stock_symbol"].as_str());
                }
            }
        }
        let (status, json) =
            get("/api/v1/market/dividend-yield-ranking?industry_id=2147483647").await;
        if status == StatusCode::OK {
            assert_eq!(json["stocks"], serde_json::json!([]));
        }
    }

    /// Phase 3 選股的真實資料庫整合測試。
    ///
    /// 驗證空的 `all` 查詢在 SQL 前回 422，以及以 `twse` 作為有效條件時能
    /// 執行每股最新資料查詢並維持固定 envelope。沒有資料庫連線時安全跳過。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn phase3_screen_endpoint_db_semantics() {
        dotenvy::dotenv().ok();
        if sqlx::query("SELECT 1")
            .execute(crate::infra::database::get_connection())
            .await
            .is_err()
        {
            println!("跳過 phase3_screen_endpoint_db_semantics：無資料庫連接");
            return;
        }
        let key = std::env::var("DATA_API_KEY").unwrap_or_else(|_| {
            let generated = "phase3-integration-test-key".to_owned();
            unsafe { std::env::set_var("DATA_API_KEY", &generated) };
            generated
        });
        let get = |path: &'static str| {
            let key = key.clone();
            async move {
                let response = router()
                    .oneshot(
                        Request::get(path)
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

        let (status, _) = get("/api/v1/stocks/screen").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        let (status, json) = get(
            "/api/v1/stocks/screen?market=twse&sort_by=valuation_percentage&sort_order=desc&limit=2",
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "合法選股 SQL 應能在真實 schema 執行"
        );
        assert_eq!(json["data_as_of"], serde_json::Value::Null);
        assert!(json["stocks"].is_array());
        let stocks = json["stocks"].as_array().unwrap();
        assert!(stocks.len() <= 2);
        for stock in stocks {
            assert_eq!(stock["market_id"], 2, "twse 篩選不可混入其他市場");
            for field in ["valuation_date", "yield_date"] {
                if let Some(date) = stock[field].as_str() {
                    assert!(chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok());
                }
            }
            if let Some(month) = stock["revenue_month"].as_str() {
                assert_eq!(month.len(), 7, "營收來源月份應為 YYYY-MM");
            }
            if let Some(period) = stock["financial_period"].as_str() {
                assert!(period.contains("-Q"), "財報來源期間應為 YYYY-Qn");
            }
        }
        for pair in stocks.windows(2) {
            match (
                pair[0]["valuation_percentage"].as_f64(),
                pair[1]["valuation_percentage"].as_f64(),
            ) {
                (Some(left), Some(right)) => assert!(left >= right),
                (None, Some(_)) => panic!("DESC NULLS LAST 不可把 null 放在有效值之前"),
                _ => {}
            }
        }
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

    /// Phase 4 三個 endpoints 的真實資料庫語意整合測試（§4.8–§4.10）。
    ///
    /// 覆蓋：參數不合法 → 422（含區間顛倒、區間超過 92 天、非法 enum、
    /// limit 超界）；查無資料 → 200 空陣列（三者皆無 404 語意）；行事曆
    /// 事件日期排序與無效日期標記不產生事件；QFII 排行的排除與排序規則。
    /// 無資料庫連線時安全跳過。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn phase4_endpoints_db_semantics() {
        dotenvy::dotenv().ok();
        let pool = crate::infra::database::get_connection();
        if sqlx::query("SELECT 1").execute(pool).await.is_err() {
            println!("跳過 phase4_endpoints_db_semantics：無資料庫連接");
            return;
        }
        let key = std::env::var("DATA_API_KEY").unwrap_or_else(|_| {
            let generated = "phase4-integration-test-key".to_owned();
            unsafe { std::env::set_var("DATA_API_KEY", &generated) };
            generated
        });
        let get = |path: String| {
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

        // 語意一：參數不合法 → 422（在任何 SQL 之前擋下）。
        for path in [
            // §4.8 指數歷史。
            "/api/v1/market/index-history?from=2026-07-17&to=2026-01-01",
            "/api/v1/market/index-history?from=2026-7-1",
            "/api/v1/market/index-history?limit=0",
            "/api/v1/market/index-history?limit=366",
            // §4.9 行事曆：區間顛倒、超過 92 天、非法 enum、limit 超界。
            "/api/v1/market/dividend-calendar?from=2026-07-17&to=2026-07-01",
            "/api/v1/market/dividend-calendar?from=2026-01-01&to=2026-04-30",
            "/api/v1/market/dividend-calendar?event_type=cash",
            "/api/v1/market/dividend-calendar?limit=201",
            // §4.10 QFII 排行。
            "/api/v1/market/qfii-holding-ranking?market=emerging",
            "/api/v1/market/qfii-holding-ranking?sort_by=issued_share",
            "/api/v1/market/qfii-holding-ranking?industry_id=0",
            "/api/v1/market/qfii-holding-ranking?limit=51",
        ] {
            let (status, _) = get(path.to_owned()).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{path} 應回 422");
        }

        // 語意二（§4.8）：index 表最早資料為 2018 年後，1900 年區間必然
        // 無資料 → 200 空陣列、data_as_of null（無 404 語意）。
        let (status, json) =
            get("/api/v1/market/index-history?from=1900-01-01&to=1900-12-31".to_owned()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["points"], serde_json::json!([]));
        assert_eq!(json["data_as_of"], serde_json::Value::Null);

        // §4.8：正常查詢須依日期新到舊，data_as_of 為最新一筆日期。
        let (status, json) = get("/api/v1/market/index-history?limit=10".to_owned()).await;
        assert_eq!(status, StatusCode::OK);
        let points = json["points"].as_array().expect("points array");
        if let Some(first) = points.first() {
            assert_eq!(json["data_as_of"], first["date"]);
        }
        for pair in points.windows(2) {
            assert!(
                pair[0]["date"].as_str() > pair[1]["date"].as_str(),
                "指數歷史必須依日期由新到舊"
            );
        }

        // 語意三（§4.9）：1900 年代不可能有除權息事件 → 200 空陣列。
        let (status, json) =
            get("/api/v1/market/dividend-calendar?from=1900-01-01&to=1900-03-31".to_owned()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["events"], serde_json::json!([]));
        assert_eq!(json["data_as_of"], serde_json::Value::Null);

        // §4.9：找一個實際有除息事件的區間驗證排序與日期合法性。以資料庫
        // 中最大的合法除息日為錨點，往前 30 天，保證區間內至少一筆事件。
        let anchor: Option<String> = sqlx::query_scalar(
            r#"SELECT MAX("ex-dividend_date1") FROM dividend
               WHERE "ex-dividend_date1" ~ '^\d{4}-\d{2}-\d{2}$'"#,
        )
        .fetch_one(pool)
        .await
        .expect("anchor query should work");
        if let Some(anchor) = anchor {
            let to = chrono::NaiveDate::parse_from_str(&anchor, "%Y-%m-%d").expect("anchor date");
            let from = to - chrono::Days::new(30);
            let (status, json) = get(format!(
                "/api/v1/market/dividend-calendar?from={from}&to={to}&limit=200"
            ))
            .await;
            assert_eq!(status, StatusCode::OK);
            let events = json["events"].as_array().expect("events array");
            assert!(!events.is_empty(), "錨點區間內至少應有一筆除息事件");
            for event in events {
                // 每筆事件日期都必須是合法日期且落在查詢區間內——這同時
                // 證明 `-`、`尚未公布` 等無效標記不會產生事件。
                let date = chrono::NaiveDate::parse_from_str(
                    event["event_date"].as_str().expect("event_date string"),
                    "%Y-%m-%d",
                )
                .expect("event_date 必須是合法日期");
                assert!((from..=to).contains(&date), "事件日期必須落在查詢區間");
                assert!(matches!(
                    event["event_type"].as_str(),
                    Some("ex_dividend" | "ex_rights" | "cash_payable" | "stock_payable")
                ));
                assert!(matches!(
                    event["quarter"].as_str(),
                    Some("A" | "H1" | "H2" | "Q1" | "Q2" | "Q3" | "Q4")
                ));
            }
            // 行事曆語意：event_date ASC、同日 stock_symbol ASC。
            for pair in events.windows(2) {
                let left = (
                    pair[0]["event_date"].as_str().unwrap(),
                    pair[0]["stock_symbol"].as_str().unwrap(),
                );
                let right = (
                    pair[1]["event_date"].as_str().unwrap(),
                    pair[1]["stock_symbol"].as_str().unwrap(),
                );
                assert!(left <= right, "行事曆必須依日期升冪、同日依代號升冪");
            }
            // event_type 過濾：單一類型查詢不得混入其他事件。
            let (status, json) = get(format!(
                "/api/v1/market/dividend-calendar?from={from}&to={to}&event_type=ex_dividend&limit=200"
            ))
            .await;
            assert_eq!(status, StatusCode::OK);
            assert!(
                json["events"]
                    .as_array()
                    .expect("events array")
                    .iter()
                    .all(|event| event["event_type"] == "ex_dividend")
            );
        }

        // 語意四（§4.10）：查無資料的產業 → 200 空陣列；正常查詢驗證排除
        // 與兩種排序。data_as_of 固定 null（快照無列級日期，不可偽造）。
        let (status, json) =
            get("/api/v1/market/qfii-holding-ranking?industry_id=2147483647".to_owned()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["stocks"], serde_json::json!([]));
        assert_eq!(json["data_as_of"], serde_json::Value::Null);
        for sort_by in ["percentage", "shares"] {
            let (status, json) = get(format!(
                "/api/v1/market/qfii-holding-ranking?sort_by={sort_by}&limit=50"
            ))
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(json["data_as_of"], serde_json::Value::Null);
            let stocks = json["stocks"].as_array().expect("stocks array");
            for (index, stock) in stocks.iter().enumerate() {
                assert_eq!(stock["rank"], index as u64 + 1, "名次必須從一連續遞增");
                assert!(
                    matches!(stock["market_id"].as_i64(), Some(2 | 4)),
                    "all 只含上市與上櫃"
                );
                assert_ne!(stock["qfii_shares_held"], 0, "零持股必須被排除");
            }
            let metric = match sort_by {
                "percentage" => "qfii_share_holding_percentage",
                _ => "qfii_shares_held",
            };
            for pair in stocks.windows(2) {
                let left = pair[0][metric].as_f64().unwrap();
                let right = pair[1][metric].as_f64().unwrap();
                assert!(left >= right, "{sort_by} 必須由高到低");
                if left == right {
                    assert!(pair[0]["stock_symbol"].as_str() <= pair[1]["stock_symbol"].as_str());
                }
            }
        }
        // twse 過濾不可混入其他市場。
        let (status, json) =
            get("/api/v1/market/qfii-holding-ranking?market=twse".to_owned()).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            json["stocks"]
                .as_array()
                .expect("stocks array")
                .iter()
                .all(|stock| stock["market_id"] == 2)
        );
    }

    /// 漲跌幅／成交量排行的真實資料庫語意整合測試（movers 計畫 §4）。
    ///
    /// 這個測試跑在**非交易時段的收盤來源路徑**上：即時快取為空時
    /// endpoint 必須回 `source = "closing"`，並且各欄位的單位與缺值語意
    /// 完全依 §4.4 決定。覆蓋參數 422、三種排序、市場過濾、名次連續性與
    /// 「收盤來源不得出現張數／快照時間」等契約。無資料庫連線時安全跳過。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn movers_endpoint_db_semantics() {
        dotenvy::dotenv().ok();
        let pool = crate::infra::database::get_connection();
        if sqlx::query("SELECT 1").execute(pool).await.is_err() {
            println!("跳過 movers_endpoint_db_semantics：無資料庫連接");
            return;
        }
        // 這個測試驗證的是「非交易時段」語意，若本機殘留即時快照會讓
        // 來源變成 realtime，因此先確認快取為空，否則直接跳過而不是清空
        // 別人的快取（清空會影響正在跑的追蹤任務）。
        if !crate::infra::cache::SHARE.stock_snapshots_are_empty() {
            println!("跳過 movers_endpoint_db_semantics：即時快照非空（目前為盤中時段）");
            return;
        }
        let key = std::env::var("DATA_API_KEY").unwrap_or_else(|_| {
            let generated = "movers-integration-test-key".to_owned();
            unsafe { std::env::set_var("DATA_API_KEY", &generated) };
            generated
        });
        let get = |path: String| {
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

        // 語意一：參數不合法 → 422（在任何 SQL 之前擋下）。
        for path in [
            "/api/v1/market/movers?rank_by=top_trade_value",
            "/api/v1/market/movers?rank_by=TOP_GAINERS",
            "/api/v1/market/movers?market=emerging",
            "/api/v1/market/movers?limit=0",
            "/api/v1/market/movers?limit=51",
        ] {
            let (status, _) = get(path.to_owned()).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{path} 應回 422");
        }

        // 語意二：非交易時段固定走收盤來源，且缺值語意依 §4.4。
        let (status, json) = get("/api/v1/market/movers?limit=20".to_owned()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["source"], "closing");
        assert_eq!(json["is_realtime"], false);
        assert_eq!(json["rank_by"], "top_gainers");
        assert_eq!(json["market"], "all");
        assert_eq!(
            json["snapshot_updated_at"],
            serde_json::Value::Null,
            "收盤來源沒有快照時間，不可偽造"
        );
        let data_as_of = json["data_as_of"].as_str().expect("data_as_of 應為字串");
        assert_eq!(data_as_of.len(), 10, "data_as_of 應為 YYYY-MM-DD");
        let movers = json["movers"].as_array().expect("movers array");
        assert!(movers.len() <= 20);
        for (index, mover) in movers.iter().enumerate() {
            assert_eq!(mover["rank"], index as u64 + 1, "名次必須從一連續遞增");
            assert!(
                matches!(mover["market_id"].as_i64(), Some(2 | 4)),
                "all 只含上市與上櫃"
            );
            assert!(
                mover["volume_shares"].as_f64().unwrap_or_default() > 0.0,
                "零成交量必須被排除"
            );
            // 收盤來源沒有「張」與昨收，也沒有採集站點。
            assert_eq!(mover["volume_lots"], serde_json::Value::Null);
            assert_eq!(mover["last_close"], serde_json::Value::Null);
            assert_eq!(mover["source_site"], serde_json::Value::Null);
        }

        // 語意三：三種排序鍵的方向與同值穩定排序。
        for (rank_by, metric, descending) in [
            ("top_gainers", "change_percent", true),
            ("top_losers", "change_percent", false),
            ("top_volume", "volume_shares", true),
        ] {
            let (status, json) =
                get(format!("/api/v1/market/movers?rank_by={rank_by}&limit=50")).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(json["rank_by"], rank_by);
            let movers = json["movers"].as_array().expect("movers array");
            for pair in movers.windows(2) {
                let left = pair[0][metric].as_f64().expect("指標應為數值");
                let right = pair[1][metric].as_f64().expect("指標應為數值");
                if descending {
                    assert!(left >= right, "{rank_by} 必須由高到低");
                } else {
                    assert!(left <= right, "{rank_by} 必須由低到高");
                }
                if left == right {
                    assert!(
                        pair[0]["stock_symbol"].as_str() <= pair[1]["stock_symbol"].as_str(),
                        "{rank_by} 同值時必須依股票代號升冪"
                    );
                }
            }
        }

        // 語意四：市場過濾不得混入其他市場。
        for (market, expected_id) in [("twse", 2), ("tpex", 4)] {
            let (status, json) = get(format!("/api/v1/market/movers?market={market}")).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(json["market"], market);
            assert!(
                json["movers"]
                    .as_array()
                    .expect("movers array")
                    .iter()
                    .all(|mover| mover["market_id"] == expected_id)
            );
        }
    }
}
