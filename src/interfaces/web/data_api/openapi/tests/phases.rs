//! Phase 1～4 path（個股基本資料、財報、估值、市場輔助）的 OpenAPI 契約。

use utoipa::OpenApi;

use super::super::ApiDoc;
use super::{assert_endpoint_responses, enum_values, get_operation, query_schema};

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
    assert_eq!(
        query_schema(operation, "min_operating_margin_percent")["maximum"],
        100
    );
    assert!(
        query_schema(operation, "margins_yoy_up")
            .to_string()
            .contains("boolean")
    );
    let sort_by = enum_values(&document, query_schema(operation, "sort_by"));
    for field in [
        "f_score",
        "gross_margin",
        "debt_ratio",
        "operating_margin",
        "pe_ratio",
        "pb_ratio",
    ] {
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
        ("operating_margin_percent", "number"),
        ("net_margin_percent", "number"),
        ("margins_yoy_up", "boolean"),
        ("margin_period", "string"),
        ("rs_score", "integer"),
        ("foreign_streak", "integer"),
        ("trust_streak", "integer"),
        ("avg_trade_value_20d", "number"),
        ("pe_ratio", "number"),
        ("pb_ratio", "number"),
        ("ratio_date", "string"),
    ] {
        let schema = screened[field].to_string();
        assert!(
            schema.contains(kind) && schema.contains("null"),
            "{field}：{schema}"
        );
    }

    // 法人連續買超門檻 1–60；回應附籌碼資料日。
    for name in ["min_foreign_streak", "min_trust_streak"] {
        let schema = query_schema(operation, name);
        assert_eq!(schema["minimum"], 1, "{name}");
        assert_eq!(schema["maximum"], 60, "{name}");
    }
    let liquidity = query_schema(operation, "min_avg_trade_value");
    assert_eq!(
        (liquidity["minimum"].as_u64(), liquidity["maximum"].as_u64()),
        (Some(1), Some(100_000))
    );
    let response = &document["components"]["schemas"]["StockScreeningResponse"]["properties"];
    assert!(response["chip_as_of"].to_string().contains("null"));
    // 本益比上限 0–10000、股價淨值比上限 0–1000。
    assert_eq!(query_schema(operation, "max_pe")["maximum"], 10000);
    assert_eq!(query_schema(operation, "max_pb")["maximum"], 1000);
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
