//! OpenAPI 文件的契約測試：路徑、參數範圍、列舉值與回應欄位。

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
    let reference = operation["responses"][status]["content"]["application/json"]["schema"]["$ref"]
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

/// utoipa 6 對「inline 列舉＋default」會產生 `allOf: [列舉, {type: object, default}]`，
/// 要求參數同時是字串又是物件；有預設值的列舉參數因此改用 `$ref`（default 為同層欄位），
/// 這裡確保所有 query 參數都不再出現這種互相矛盾的 schema。
#[test]
fn query_parameters_have_no_contradictory_all_of() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let mut checked = 0;
    for (path, item) in document["paths"].as_object().expect("paths") {
        for parameter in item["get"]["parameters"].as_array().into_iter().flatten() {
            let schema = parameter["schema"].to_string();
            assert!(
                !(schema.contains("allOf") && schema.contains(r#""type":"object""#)),
                "{path} {} 的 schema 互相矛盾：{schema}",
                parameter["name"]
            );
            checked += 1;
        }
    }
    assert!(checked > 0);

    // 有預設值的列舉參數是 `$ref` 加上同層 default，列舉值由 components 提供。
    let rank_by = query_schema(get_operation(&document, "/api/v1/market/movers"), "rank_by");
    assert_eq!(rank_by["$ref"], "#/components/schemas/MoversRankByValue");
    assert_eq!(rank_by["default"], "top_gainers");
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
        "/api/v1/stocks/{symbol}/financial-reports",
        "/api/v1/stocks/{symbol}/financial-health",
        "/api/v1/stocks/{symbol}/dividends",
        "/api/v1/stocks/{symbol}/dividend-fill",
        "/api/v1/stocks/{symbol}/valuation",
        "/api/v1/stocks/{symbol}/chip",
        "/api/v1/market/breadth",
        "/api/v1/market/dividend-yield-ranking",
        "/api/v1/stocks/screen",
        "/api/v1/market/index-history",
        "/api/v1/market/dividend-calendar",
        "/api/v1/market/qfii-holding-ranking",
        "/api/v1/market/movers",
        "/api/v1/market/industries",
        "/api/v1/stocks/{symbol}/technicals",
        "/api/v1/market/signals",
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
    let point = &document["components"]["schemas"]["MarketBreadth"]["properties"];
    for field in ["new_52_week_high", "new_52_week_low"] {
        assert_eq!(point[field]["type"], "integer", "{field}");
    }
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

    // 財務體質條件：F 分數 0–9、毛利率上限 100、負債比 0–1000、自由現金流布林；
    // 排序多出 f_score／gross_margin／debt_ratio。
    assert_eq!(query_schema(operation, "min_f_score")["minimum"], 0);
    assert_eq!(query_schema(operation, "min_f_score")["maximum"], 9);
    assert_eq!(
        query_schema(operation, "min_gross_margin_percent")["maximum"],
        100
    );
    assert_eq!(
        query_schema(operation, "max_debt_ratio_percent")["maximum"],
        1000
    );
    assert!(
        query_schema(operation, "positive_free_cash_flow")
            .to_string()
            .contains("boolean")
    );
    let sort_by = enum_values(&document, query_schema(operation, "sort_by"));
    for field in ["f_score", "gross_margin", "debt_ratio"] {
        assert!(
            sort_by
                .as_array()
                .is_some_and(|values| values.contains(&field.into())),
            "sort_by 缺少 {field}：{sort_by}"
        );
    }
    let screened = &document["components"]["schemas"]["ScreenedStock"]["properties"];
    for (field, kind) in [
        ("f_score", "integer"),
        ("f_score_evaluated", "integer"),
        ("f_score_year", "integer"),
        ("gross_margin_percent", "number"),
        ("debt_ratio_percent", "number"),
        ("free_cash_flow_ttm", "integer"),
        ("health_period", "string"),
    ] {
        let schema = screened[field].to_string();
        assert!(
            schema.contains(kind) && schema.contains("null"),
            "{field}：{schema}"
        );
    }
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
    let properties = &document["components"]["schemas"]["MarketIndexHistoryResponse"]["properties"];
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
    let properties = &document["components"]["schemas"]["DividendCalendarResponse"]["properties"];
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
    let properties = &document["components"]["schemas"]["QfiiHoldingRankingResponse"]["properties"];
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
    let symbol_properties = &document["components"]["schemas"]["CagrSymbolResponse"]["properties"];
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

/// 三大報表 path 的契約：期間 enum 與預設值、limit 範圍、三個陣列的 item，
/// 以及「金額是整數、每股數值是可為 null 的 number」。
#[test]
fn openapi_financial_reports_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/stocks/{symbol}/financial-reports");
    assert_endpoint_responses(operation, "FinancialReportResponse", true);
    let period = query_schema(operation, "period_type");
    assert_eq!(period["default"], "quarterly");
    assert_eq!(
        enum_values(&document, period),
        serde_json::json!(["quarterly", "annual"])
    );
    let limit = query_schema(operation, "limit");
    assert_eq!(limit["default"], 8);
    assert_eq!(limit["minimum"], 1);
    assert_eq!(limit["maximum"], 40);

    let schemas = &document["components"]["schemas"];
    let properties = &schemas["FinancialReportResponse"]["properties"];
    for (list, item) in [
        ("income_statements", "IncomeStatementItem"),
        ("balance_sheets", "BalanceSheetItem"),
        ("cash_flow_statements", "CashFlowStatementItem"),
    ] {
        assert_eq!(properties[list]["type"], "array");
        assert_eq!(
            properties[list]["items"]["$ref"],
            format!("#/components/schemas/{item}")
        );
        assert_eq!(
            schemas[item]["properties"]["fiscal_year"]["type"],
            "integer"
        );
        assert_eq!(schemas[item]["properties"]["source"]["type"], "string");
    }
    assert_eq!(properties["amount_unit"]["type"], "string");
    assert!(properties["data_as_of"].to_string().contains("null"));

    let income = &schemas["IncomeStatementItem"]["properties"];
    let revenue = income["revenue"].to_string();
    assert!(revenue.contains("integer") && revenue.contains("null"));
    let eps = income["eps"].to_string();
    assert!(eps.contains("number") && eps.contains("null"));
    let balance = &schemas["BalanceSheetItem"]["properties"];
    assert!(balance["total_assets"].to_string().contains("integer"));
    assert!(
        balance["book_value_per_share"]
            .to_string()
            .contains("number")
    );
    let cash_flow = &schemas["CashFlowStatementItem"]["properties"];
    assert!(cash_flow["free_cash_flow"].to_string().contains("integer"));
}

/// 財務體質 path 的契約：fiscal_year 範圍、9 項指標的 item 與分類／單位列舉，
/// 以及「得分可為 null、數值是可為 null 的 number」。
#[test]
fn openapi_financial_health_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/stocks/{symbol}/financial-health");
    assert_endpoint_responses(operation, "FinancialHealthResponse", true);
    let fiscal_year = query_schema(operation, "fiscal_year");
    assert_eq!(fiscal_year["minimum"], 2000);
    assert_eq!(fiscal_year["maximum"], 2100);

    let schemas = &document["components"]["schemas"];
    let properties = &schemas["FinancialHealthResponse"]["properties"];
    assert_eq!(properties["criteria"]["type"], "array");
    assert_eq!(
        properties["criteria"]["items"]["$ref"],
        "#/components/schemas/HealthCriterion"
    );
    for field in ["score", "evaluated", "max_score"] {
        assert_eq!(properties[field]["type"], "integer", "{field}");
    }
    for field in ["fiscal_year", "compared_with"] {
        assert!(properties[field].to_string().contains("null"), "{field}");
    }

    let criterion = &schemas["HealthCriterion"]["properties"];
    assert!(criterion["passed"].to_string().contains("boolean"));
    assert!(criterion["passed"].to_string().contains("null"));
    for field in ["value", "baseline"] {
        let schema = criterion[field].to_string();
        assert!(
            schema.contains("number") && schema.contains("null"),
            "{field}"
        );
    }
    assert_eq!(
        enum_values(&document, &criterion["category"]),
        serde_json::json!([
            "profitability",
            "leverage_liquidity",
            "operating_efficiency"
        ])
    );
    assert_eq!(
        enum_values(&document, &criterion["unit"]),
        serde_json::json!(["ratio", "thousand_twd"])
    );
}

/// 估值歷史區間 path 的契約：years 範圍與預設、兩個倍數共用 RatioBandItem、可為 null 的欄位。
#[test]
fn openapi_valuation_band_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/stocks/{symbol}/valuation-band");
    assert_endpoint_responses(operation, "ValuationBandResponse", true);
    let years = query_schema(operation, "years");
    assert_eq!(years["default"], 5);
    assert_eq!(years["minimum"], 1);
    assert_eq!(years["maximum"], 10);

    let schemas = &document["components"]["schemas"];
    let response = &schemas["ValuationBandResponse"]["properties"];
    for field in ["price_earning_ratio", "price_to_book_ratio"] {
        let schema = response[field].to_string();
        assert!(schema.contains("RatioBandItem"), "{field}");
        assert!(schema.contains("null"), "{field}");
    }
    let band = &schemas["RatioBandItem"]["properties"];
    for field in ["current", "percentile", "implied_prices"] {
        assert!(band[field].to_string().contains("null"), "{field}");
    }
    assert_eq!(band["samples"]["type"], "integer");
    assert_eq!(band["excluded_days"]["type"], "integer");
    assert!(
        schemas["ImpliedPriceBand"]["properties"]["median"]
            .to_string()
            .contains("number")
    );
}

/// 技術指標 path 的契約：days 範圍與預設、date 參數、可為 null 的指標欄位。
#[test]
fn openapi_technicals_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/stocks/{symbol}/technicals");
    assert_endpoint_responses(operation, "TechnicalIndicatorsResponse", true);
    let days = query_schema(operation, "days");
    assert_eq!(days["default"], 20);
    assert_eq!(days["minimum"], 1);
    assert_eq!(days["maximum"], 120);
    assert!(
        query_schema(operation, "date")
            .to_string()
            .contains("string")
    );

    let schemas = &document["components"]["schemas"];
    let point = &schemas["TechnicalPoint"]["properties"];
    for field in [
        "ma5",
        "ma240",
        "bias20",
        "rsi14",
        "k",
        "d",
        "macd_dif",
        "macd_histogram",
    ] {
        assert!(point[field].to_string().contains("null"), "{field}");
    }
    assert!(point["close"]["type"].to_string().contains("number"));
    let response = &schemas["TechnicalIndicatorsResponse"]["properties"];
    assert!(response["data_as_of"].to_string().contains("null"));
    assert_eq!(response["adjusted"]["type"], "boolean");
    assert!(
        response["signals"]
            .to_string()
            .contains("TechnicalSignalItem")
    );
}

/// 技術訊號選股 path 的契約：limit 範圍、kind／indicator 列舉值與回應欄位。
#[test]
fn openapi_signals_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/market/signals");
    assert_endpoint_responses(operation, "MarketSignalsResponse", true);
    let limit = query_schema(operation, "limit");
    assert_eq!(limit["default"], 50);
    assert_eq!(limit["minimum"], 1);
    assert_eq!(limit["maximum"], 200);
    assert!(
        query_schema(operation, "kind")
            .to_string()
            .contains("SignalKindValue")
    );
    assert!(
        query_schema(operation, "indicator")
            .to_string()
            .contains("SignalIndicatorValue")
    );

    let schemas = &document["components"]["schemas"];
    let kinds = schemas["SignalKindValue"].to_string();
    for kind in ["golden_cross", "cross_below", "new_high", "new_low"] {
        assert!(kinds.contains(kind), "{kind}");
    }
    let indicators = schemas["SignalIndicatorValue"].to_string();
    for indicator in ["ma5_ma20", "close_ma240", "rsi14", "52_week"] {
        assert!(indicators.contains(indicator), "{indicator}");
    }
    let item = &schemas["SignalStock"]["properties"];
    for field in ["closing_price", "change_percent", "trade_value"] {
        assert!(item[field].to_string().contains("null"), "{field}");
    }
    let response = &schemas["MarketSignalsResponse"]["properties"];
    assert_eq!(response["adjusted"]["type"], "boolean");
    assert!(response["kind"].to_string().contains("null"));
    assert!(response["summary"].to_string().contains("SignalCount"));
    assert!(response["items"].to_string().contains("SignalStock"));
}

/// 訊號績效與個股訊號歷史 path 的契約：參數範圍、列舉值與回應欄位。
#[test]
fn openapi_signal_history_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/market/signals/performance");
    assert_endpoint_responses(operation, "SignalPerformanceResponse", true);
    for name in ["from", "to", "industry"] {
        assert!(query_schema(operation, name).is_object(), "{name}");
    }
    assert!(
        query_schema(operation, "kind")
            .to_string()
            .contains("SignalKindValue")
    );

    let operation = get_operation(&document, "/api/v1/stocks/{symbol}/signals");
    assert_endpoint_responses(operation, "StockSignalsResponse", true);
    let days = query_schema(operation, "days");
    assert_eq!(days["default"], 180);
    assert_eq!(days["minimum"], 1);
    assert_eq!(days["maximum"], 730);
    let limit = query_schema(operation, "limit");
    assert_eq!(limit["default"], 50);
    assert_eq!(limit["maximum"], 200);

    let schemas = &document["components"]["schemas"];
    let horizon = &schemas["HorizonStat"]["properties"];
    for field in [
        "avg_return",
        "median_return",
        "win_rate",
        "avg_excess_return",
        "beat_market_rate",
    ] {
        assert!(horizon[field].to_string().contains("null"), "{field}");
    }
    let response = &schemas["SignalPerformanceResponse"]["properties"];
    assert_eq!(response["adjusted"]["type"], "boolean");
    assert!(response["benchmark"].to_string().contains("BenchmarkStat"));
    assert!(
        response["items"]
            .to_string()
            .contains("SignalPerformanceItem")
    );
    let item = &schemas["StockSignalItem"]["properties"];
    for field in ["return_5d", "return_20d", "return_60d", "closing_price"] {
        assert!(item[field].to_string().contains("null"), "{field}");
    }
}

/// 產業表現 path 的契約：days 範圍與預設、market 列舉、可為 null 的統計欄位。
#[test]
fn openapi_industries_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/market/industries");
    assert_endpoint_responses(operation, "IndustryPerformanceResponse", true);
    let days = query_schema(operation, "days");
    assert_eq!(days["default"], 1);
    assert_eq!(days["minimum"], 1);
    assert_eq!(days["maximum"], 60);
    assert_eq!(
        query_schema(operation, "market")["$ref"],
        "#/components/schemas/MarketParamValue"
    );

    let schemas = &document["components"]["schemas"];
    let response = &schemas["IndustryPerformanceResponse"]["properties"];
    assert!(response["base_date"].to_string().contains("null"));
    assert_eq!(
        response["industries"]["items"]["$ref"],
        "#/components/schemas/IndustryPerformance"
    );
    let item = &schemas["IndustryPerformance"]["properties"];
    for field in [
        "median_change_percent",
        "cap_weighted_change_percent",
        "trade_value_share_percent",
        "median_price_earning_ratio",
        "top_gainer",
        "top_loser",
    ] {
        assert!(item[field].to_string().contains("null"), "{field}");
    }
}

/// 同業比較 path 的契約：limit 範圍與預設、target 可為 null、指標與同業陣列。
#[test]
fn openapi_peers_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/stocks/{symbol}/peers");
    assert_endpoint_responses(operation, "PeersResponse", true);
    let limit = query_schema(operation, "limit");
    assert_eq!(limit["default"], 20);
    assert_eq!(limit["minimum"], 1);
    assert_eq!(limit["maximum"], 100);

    let schemas = &document["components"]["schemas"];
    let response = &schemas["PeersResponse"]["properties"];
    assert!(response["target"].to_string().contains("null"));
    assert_eq!(
        response["metrics"]["items"]["$ref"],
        "#/components/schemas/PeerMetric"
    );
    assert_eq!(
        response["peers"]["items"]["$ref"],
        "#/components/schemas/PeerStock"
    );
    let metric = &schemas["PeerMetric"]["properties"];
    for field in ["target", "industry_median", "rank"] {
        assert!(metric[field].to_string().contains("null"), "{field}");
    }
    let stock = &schemas["PeerStock"]["properties"];
    for field in [
        "market_cap",
        "price_earning_ratio",
        "f_score",
        "revenue_yoy_percent",
    ] {
        assert!(stock[field].to_string().contains("null"), "{field}");
    }
}

/// 持股總覽 path 的契約：member_id 下限與預設、產業與持股陣列、可為 null 的加權指標。
#[test]
fn openapi_portfolio_summary_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/portfolio/summary");
    assert_endpoint_responses(operation, "PortfolioSummaryResponse", false);
    let member = query_schema(operation, "member_id");
    assert_eq!(member["default"], 0);
    assert_eq!(member["minimum"], 0);

    let schemas = &document["components"]["schemas"];
    let response = &schemas["PortfolioSummaryResponse"]["properties"];
    assert_eq!(
        response["holdings"]["items"]["$ref"],
        "#/components/schemas/PortfolioHolding"
    );
    assert_eq!(
        response["industries"]["items"]["$ref"],
        "#/components/schemas/PortfolioIndustry"
    );
    for field in [
        "weighted_dividend_yield",
        "weighted_f_score",
        "unrealized_gain_percent",
    ] {
        assert!(response[field].to_string().contains("null"), "{field}");
    }
    let holding = &schemas["PortfolioHolding"]["properties"];
    for field in ["closing_price", "market_value", "weight_percent", "f_score"] {
        assert!(holding[field].to_string().contains("null"), "{field}");
    }
}

/// 持股績效 path 的契約：member_id／months 範圍與預設、月績效陣列、可為 null 的欄位。
#[test]
fn openapi_portfolio_history_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/portfolio/history");
    assert_endpoint_responses(operation, "PortfolioHistoryResponse", false);
    let months = query_schema(operation, "months");
    assert_eq!(months["default"], 12);
    assert_eq!(months["minimum"], 1);
    assert_eq!(months["maximum"], 120);
    assert_eq!(query_schema(operation, "member_id")["default"], 0);

    let schemas = &document["components"]["schemas"];
    let response = &schemas["PortfolioHistoryResponse"]["properties"];
    assert_eq!(
        response["months"]["items"]["$ref"],
        "#/components/schemas/PortfolioMonth"
    );
    for field in [
        "return_percent",
        "annualized_return_percent",
        "max_drawdown_percent",
        "benchmark_return_percent",
        "excess_return_percent",
    ] {
        assert!(response[field].to_string().contains("null"), "{field}");
    }
    let month = &schemas["PortfolioMonth"]["properties"];
    assert!(
        month["benchmark_return_percent"]
            .to_string()
            .contains("null")
    );
    assert!(month["return_percent"].to_string().contains("number"));
}

/// 股利現金流預估 path 的契約：months 範圍與預設、依據列舉、月份與項目陣列。
#[test]
fn openapi_dividend_forecast_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/portfolio/dividend-forecast");
    assert_endpoint_responses(operation, "DividendForecastResponse", false);
    let months = query_schema(operation, "months");
    assert_eq!(months["default"], 12);
    assert_eq!(months["maximum"], 24);

    let schemas = &document["components"]["schemas"];
    let response = &schemas["DividendForecastResponse"]["properties"];
    assert_eq!(
        response["items"]["items"]["$ref"],
        "#/components/schemas/DividendForecastItem"
    );
    assert!(
        response["forward_yield_percent"]
            .to_string()
            .contains("null")
    );
    let item = &schemas["DividendForecastItem"]["properties"];
    assert_eq!(
        enum_values(&document, &item["basis"]),
        serde_json::json!(["announced", "amount_announced", "estimated"])
    );
}

/// 填息分析 path 的契約：limit 範圍與預設、events／summary 結構與可為 null 的欄位。
#[test]
fn openapi_dividend_fill_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/stocks/{symbol}/dividend-fill");
    assert_endpoint_responses(operation, "DividendFillResponse", true);
    let limit = query_schema(operation, "limit");
    assert_eq!(limit["default"], 12);
    assert_eq!(limit["minimum"], 1);
    assert_eq!(limit["maximum"], 40);

    let schemas = &document["components"]["schemas"];
    let response = &schemas["DividendFillResponse"]["properties"];
    assert_eq!(response["events"]["type"], "array");
    assert_eq!(
        response["events"]["items"]["$ref"],
        "#/components/schemas/DividendFillEvent"
    );
    assert!(
        response["summary"]
            .to_string()
            .contains("DividendFillSummary")
    );
    let event = &schemas["DividendFillEvent"]["properties"];
    assert_eq!(event["filled"]["type"], "boolean");
    assert_eq!(event["trading_days_since_ex"]["type"], "integer");
    for field in ["fill_date", "trading_days_to_fill", "gap_percent"] {
        assert!(event[field].to_string().contains("null"), "{field}");
    }
    let summary = &schemas["DividendFillSummary"]["properties"];
    assert!(
        summary["average_trading_days_to_fill"]
            .to_string()
            .contains("number")
    );
}
