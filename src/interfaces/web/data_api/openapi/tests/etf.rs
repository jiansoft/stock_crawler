//! ETF 專區清單與明細 path 的 OpenAPI 契約。

use utoipa::OpenApi;

use super::super::ApiDoc;
use super::{assert_endpoint_responses, enum_values, get_operation, query_schema};

/// ETF 清單 path 的契約：排序鍵與配息頻率列舉、limit 範圍與預設、可為 null 的欄位。
#[test]
fn openapi_etf_list_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/etfs");
    assert_endpoint_responses(operation, "EtfListResponse", false);
    let sort = query_schema(operation, "sort");
    assert_eq!(sort["default"], "trade_value");
    assert_eq!(
        enum_values(&document, sort),
        serde_json::json!([
            "trade_value",
            "total_assets",
            "premium_rate",
            "yield_12m",
            "net_flow",
            "ret_1m",
            "ret_ytd",
            "ret_1y",
            "holders"
        ])
    );
    assert_eq!(
        enum_values(&document, query_schema(operation, "frequency")),
        serde_json::json!(["monthly", "quarterly", "semi_annual", "annual", "none"])
    );
    let limit = query_schema(operation, "limit");
    assert_eq!(
        (
            limit["default"].clone(),
            limit["minimum"].clone(),
            limit["maximum"].clone()
        ),
        (20.into(), 1.into(), 100.into())
    );
    let schemas = &document["components"]["schemas"];
    assert_eq!(
        schemas["EtfListResponse"]["properties"]["items"]["items"]["$ref"],
        "#/components/schemas/EtfItem"
    );
    for field in ["premium_rate", "yield_12m", "net_flow", "next_ex_date"] {
        assert!(
            schemas["EtfItem"]["properties"][field]
                .to_string()
                .contains("null"),
            "{field}"
        );
    }
}

/// ETF 明細 path 的契約：404、nav_days 範圍、各區塊欄位。
#[test]
fn openapi_etf_detail_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/etfs/{symbol}");
    assert_endpoint_responses(operation, "EtfDetailResponse", true);
    let nav_days = query_schema(operation, "nav_days");
    assert_eq!(
        (nav_days["minimum"].clone(), nav_days["maximum"].clone()),
        (1.into(), 60.into())
    );
    let response = &document["components"]["schemas"]["EtfDetailResponse"]["properties"];
    for field in [
        "item",
        "profile",
        "performance",
        "holdings",
        "industries",
        "assets",
        "navs",
    ] {
        assert!(response.get(field).is_some(), "缺少 {field}");
    }
    assert_eq!(
        response["navs"]["items"]["$ref"],
        "#/components/schemas/EtfNavPoint"
    );
}
