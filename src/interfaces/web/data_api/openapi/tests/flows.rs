//! 法人買賣超、法人連續買賣超後報酬、產業資金流向與融資融券排行 path 的 OpenAPI 契約。

use utoipa::OpenApi;

use super::super::ApiDoc;
use super::{assert_endpoint_responses, enum_values, get_operation, query_schema};

/// 法人買賣超排行 path 的契約：days／limit 範圍與預設、法人別與方向列舉、可為 null 的欄位。
#[test]
fn openapi_institutional_flow_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/market/institutional-flow");
    assert_endpoint_responses(operation, "InstitutionalFlowResponse", true);
    for (name, default, maximum) in [("days", 1, 60), ("limit", 30, 100)] {
        let schema = query_schema(operation, name);
        assert_eq!(schema["default"], default, "{name}");
        assert_eq!(schema["minimum"], 1, "{name}");
        assert_eq!(schema["maximum"], maximum, "{name}");
    }
    for (name, schema, default) in [
        ("investor", "InvestorParamValue", "foreign"),
        ("side", "FlowSideParamValue", "buy"),
        ("market", "MarketParamValue", "all"),
    ] {
        let parameter = query_schema(operation, name);
        assert_eq!(parameter["$ref"], format!("#/components/schemas/{schema}"));
        assert_eq!(parameter["default"], default, "{name}");
    }

    let schemas = &document["components"]["schemas"];
    let item = &schemas["InstitutionalFlowItem"]["properties"];
    for field in ["volume_ratio", "closing_price"] {
        assert!(item[field].to_string().contains("null"), "{field}");
    }
    assert_eq!(item["streak"]["type"], "integer");
    let response = &schemas["InstitutionalFlowResponse"]["properties"];
    assert!(response["industry"].to_string().contains("null"));
    assert_eq!(
        response["items"]["items"]["$ref"],
        "#/components/schemas/InstitutionalFlowItem"
    );
}

/// 法人連續買賣超後報酬統計 path 的契約：streak 範圍與預設、法人別／方向／市場列舉、回應欄位。
#[test]
fn openapi_institutional_streak_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/market/institutional-streak/performance");
    assert_endpoint_responses(operation, "InstitutionalStreakPerformanceResponse", true);
    let streak = query_schema(operation, "streak");
    assert_eq!(streak["default"], 3);
    assert_eq!(streak["minimum"], 1);
    assert_eq!(streak["maximum"], 20);
    for (name, schema, default) in [
        ("investor", "InvestorParamValue", "foreign"),
        ("side", "FlowSideParamValue", "buy"),
        ("market", "MarketParamValue", "all"),
    ] {
        let parameter = query_schema(operation, name);
        assert_eq!(parameter["$ref"], format!("#/components/schemas/{schema}"));
        assert_eq!(parameter["default"], default, "{name}");
    }
    for name in ["from", "to", "industry"] {
        assert!(query_schema(operation, name).is_object(), "{name}");
    }

    let schemas = &document["components"]["schemas"];
    let response = &schemas["InstitutionalStreakPerformanceResponse"]["properties"];
    assert_eq!(response["adjusted"]["type"], "boolean");
    assert_eq!(response["occurrences"]["type"], "integer");
    assert!(response["industry"].to_string().contains("null"));
    assert_eq!(
        response["horizons"]["items"]["$ref"],
        "#/components/schemas/HorizonStat"
    );
    assert_eq!(
        response["benchmark"]["items"]["$ref"],
        "#/components/schemas/BenchmarkStat"
    );
}

/// 產業法人資金流向 path 的契約：days 範圍與預設、法人別與排序列舉、可為 null 的占比。
#[test]
fn openapi_industry_flow_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/market/industry-flow");
    assert_endpoint_responses(operation, "IndustryFlowResponse", true);
    let days = query_schema(operation, "days");
    assert_eq!(days["default"], 5);
    assert_eq!(days["minimum"], 1);
    assert_eq!(days["maximum"], 60);
    for (name, schema) in [
        ("investor", "InvestorParamValue"),
        ("market", "MarketParamValue"),
        ("order", "SortOrderParamValue"),
    ] {
        assert_eq!(
            query_schema(operation, name)["$ref"],
            format!("#/components/schemas/{schema}"),
            "{name}"
        );
    }

    let schemas = &document["components"]["schemas"];
    let item = &schemas["IndustryFlowItem"]["properties"];
    assert!(item["trade_value_ratio"].to_string().contains("null"));
    assert_eq!(
        item["top_buyers"]["items"]["$ref"],
        "#/components/schemas/IndustryFlowStock"
    );
    let response = &schemas["IndustryFlowResponse"]["properties"];
    assert_eq!(
        response["items"]["items"]["$ref"],
        "#/components/schemas/IndustryFlowItem"
    );
}

/// 融資融券排行 path 的契約：days／limit 範圍與預設、排序鍵列舉、可為 null 的欄位。
#[test]
fn openapi_margin_ranking_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/market/margin-ranking");
    assert_endpoint_responses(operation, "MarginRankingResponse", true);
    for (name, default, maximum) in [("days", 1, 60), ("limit", 30, 100)] {
        let schema = query_schema(operation, name);
        assert_eq!(schema["default"], default, "{name}");
        assert_eq!(schema["minimum"], 1, "{name}");
        assert_eq!(schema["maximum"], maximum, "{name}");
    }
    let rank_by = query_schema(operation, "rank_by");
    assert_eq!(rank_by["$ref"], "#/components/schemas/MarginRankByValue");
    assert_eq!(rank_by["default"], "margin_increase");
    assert_eq!(
        enum_values(&document, rank_by),
        serde_json::json!([
            "margin_increase",
            "margin_decrease",
            "short_increase",
            "short_decrease",
            "short_margin_ratio"
        ])
    );

    let schemas = &document["components"]["schemas"];
    let item = &schemas["MarginRankItem"]["properties"];
    for field in [
        "margin_change_percent",
        "short_margin_ratio",
        "closing_price",
    ] {
        assert!(item[field].to_string().contains("null"), "{field}");
    }
    let response = &schemas["MarginRankingResponse"]["properties"];
    assert_eq!(
        response["items"]["items"]["$ref"],
        "#/components/schemas/MarginRankItem"
    );
}
