//! 持股總覽與持股績效 path 的 OpenAPI 契約。

use utoipa::OpenApi;

use super::super::ApiDoc;
use super::{assert_endpoint_responses, get_operation, query_schema};

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
        "rs_as_of",
        "chip_as_of",
    ] {
        assert!(response[field].to_string().contains("null"), "{field}");
    }
    let holding = &schemas["PortfolioHolding"]["properties"];
    for field in [
        "closing_price",
        "market_value",
        "weight_percent",
        "f_score",
        "rs_score",
        "foreign_streak",
        "trust_streak",
    ] {
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
