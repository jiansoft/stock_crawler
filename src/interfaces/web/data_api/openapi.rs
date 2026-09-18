//! Data API 的 OpenAPI 3 文件定義。
//!
//! 文件內容全部由 handler 上的 `#[utoipa::path]` 與 DTO 上的 `ToSchema`
//! 生成；此檔只負責彙整 paths、components 與 Bearer 安全性宣告，供
//! `/api-docs/openapi.json` 與 `/swagger-ui` 使用。

use utoipa::OpenApi;

use super::{dto, handlers};

/// 由 handler 註解生成的 OpenAPI 3 文件。
#[derive(OpenApi)]
#[openapi(
    paths(handlers::search_stocks, handlers::latest_quote, handlers::price_history, handlers::stock_profile, handlers::realtime_snapshot, handlers::monthly_revenues, handlers::financial_statements, handlers::dividend_history, handlers::stock_valuation, handlers::market_breadth, handlers::dividend_yield_ranking, handlers::screen_stocks, handlers::market_index_history, handlers::dividend_calendar, handlers::qfii_holding_ranking, handlers::cagr_ranking, handlers::cagr_by_symbol, handlers::healthz),
    components(schemas(dto::Stock, dto::DailyQuote, dto::HistoricalQuote, dto::QuoteHistoryRecord, dto::StockProfile, dto::SearchResponse, dto::LatestQuoteResponse, dto::PriceHistoryResponse, dto::RealtimeSnapshotResponse, dto::MonthlyRevenue, dto::MonthlyRevenueResponse, dto::FinancialStatement, dto::FinancialStatementHistoryResponse, dto::Dividend, dto::DividendHistoryResponse, dto::StockValuation, dto::StockValuationResponse, dto::MarketBreadth, dto::MarketBreadthResponse, dto::DividendYieldRank, dto::DividendYieldRankingResponse, dto::ScreenedStock, dto::StockScreeningResponse, dto::MarketIndexPoint, dto::MarketIndexHistoryResponse, dto::DividendCalendarEvent, dto::DividendCalendarResponse, dto::QfiiHolding, dto::QfiiHoldingRankingResponse, dto::CagrCoverageInfo, dto::CagrSummary, dto::CagrRankingItem, dto::CagrRankingResponse, dto::CagrPeriodItem, dto::CagrSymbolResponse, dto::ErrorBody, dto::HealthResponse)),
    tags((name = "data-api", description = "唯讀股票資料查詢")),
    security(("bearer_auth" = [])),
    modifiers(&SecurityAddon)
)]
pub(super) struct ApiDoc;

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

#[cfg(test)]
mod tests {
    //! OpenAPI 產物的契約測試。
    //!
    //! 這些斷言全部從指定的 path 或 component 定位，不做全文搜尋，避免
    //! 跨 path 的假陽性；契約一旦被改動就會在此立即失敗。

    use utoipa::OpenApi;

    use super::ApiDoc;

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
            "/api/v1/market/cagr-ranking",
            "/api/v1/market/cagr-ranking/{stock_symbol}",
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

    /// M4 每日 CAGR 兩條 path 的 OpenAPI 契約：responses、白名單 enum、
    /// 預設值與陣列 item；並釘住「所有金額與比率為字串」這條前端契約。
    #[test]
    fn openapi_cagr_schemas_pin_field_names() {
        let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");

        let ranking = get_operation(&document, "/api/v1/market/cagr-ranking");
        assert_endpoint_responses(ranking, "CagrRankingResponse", true);
        let period = query_schema(ranking, "period");
        assert_eq!(period["default"], "Y1");
        assert_eq!(
            enum_values(&document, period),
            serde_json::json!(["M3", "M6", "Y1", "Y1H", "Y2", "Y3", "Y5", "Y7", "Y10"])
        );
        let metric = query_schema(ranking, "metric");
        assert_eq!(metric["default"], "total");
        assert_eq!(
            enum_values(&document, metric),
            serde_json::json!(["price", "total", "reinvested"])
        );
        assert_eq!(
            enum_values(&document, query_schema(ranking, "sort")),
            serde_json::json!(["cagr", "total_return"])
        );
        assert_eq!(query_schema(ranking, "market")["default"], "all");
        assert_eq!(query_schema(ranking, "stock_industry_id")["minimum"], 1);
        assert_eq!(query_schema(ranking, "include_incomplete")["default"], true);
        let limit = query_schema(ranking, "limit");
        assert_eq!(limit["default"], 50);
        assert_eq!(limit["minimum"], 1);
        assert_eq!(limit["maximum"], 200);
        let offset = query_schema(ranking, "offset");
        assert_eq!(offset["default"], 0);
        assert_eq!(offset["minimum"], 0);

        let properties = &document["components"]["schemas"]["CagrRankingResponse"]["properties"];
        assert_eq!(properties["items"]["type"], "array");
        assert_eq!(
            properties["items"]["items"]["$ref"],
            "#/components/schemas/CagrRankingItem"
        );
        // principal 與各種計數是整數，維持 JSON number。
        assert_eq!(properties["principal"]["type"], "integer");
        assert_eq!(properties["total"]["type"], "integer");
        // base_date／years 在整頁皆資料不足時為 null。
        assert!(properties["base_date"].to_string().contains("null"));
        assert!(properties["years"].to_string().contains("null"));

        // §M4 契約核心：Decimal 一律字串，計數一律整數。
        let coverage = &document["components"]["schemas"]["CagrCoverageInfo"]["properties"];
        assert_eq!(coverage["coverage_ratio"]["type"], "string");
        assert_eq!(coverage["universe"]["type"], "integer");
        assert_eq!(coverage["counted"]["type"], "integer");
        assert_eq!(coverage["survivorship_note"]["type"], "boolean");
        let summary = &document["components"]["schemas"]["CagrSummary"]["properties"];
        assert_eq!(summary["positive"]["type"], "integer");
        assert_eq!(summary["positive_ratio"]["type"], "string");

        let item = &document["components"]["schemas"]["CagrRankingItem"]["properties"];
        for field in [
            "base_price",
            "end_price",
            "end_shares",
            "cash_received",
            "end_value",
            "total_return_pct",
            "cagr_pct",
        ] {
            let schema = item[field].to_string();
            assert!(schema.contains("string"), "{field} 必須序列化為字串");
            assert!(schema.contains("null"), "{field} 資料不足時必須可為 null");
        }
        assert!(item["rank"].to_string().contains("null"));
        assert_eq!(item["dividend_events"]["type"], "integer");
        assert_eq!(item["data_complete"]["type"], "boolean");
        assert_eq!(item["stock_industry_id"]["type"], "integer");

        // 個股端點：items 是 CagrPeriodItem（多 period、無 rank）。
        let symbol = get_operation(&document, "/api/v1/market/cagr-ranking/{stock_symbol}");
        assert_endpoint_responses(symbol, "CagrSymbolResponse", true);
        assert_eq!(query_schema(symbol, "metric")["default"], "total");
        let symbol_properties =
            &document["components"]["schemas"]["CagrSymbolResponse"]["properties"];
        assert_eq!(symbol_properties["items"]["type"], "array");
        assert_eq!(
            symbol_properties["items"]["items"]["$ref"],
            "#/components/schemas/CagrPeriodItem"
        );
        let period_item = &document["components"]["schemas"]["CagrPeriodItem"]["properties"];
        assert_eq!(period_item["period"]["type"], "string");
        assert!(period_item["years"].to_string().contains("string"));
        assert!(
            period_item.get("rank").is_none(),
            "個股端點的項目不應有 rank"
        );
    }
}
