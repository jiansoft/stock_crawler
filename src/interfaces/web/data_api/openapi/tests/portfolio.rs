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
        "weighted_pe",
        "weighted_pb",
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
        "revenue_month",
        "revenue_yoy_percent",
        "revenue_decline_months",
        "margin_period",
        "gross_margin_percent",
        "operating_margin_percent",
        "net_margin_percent",
        "margins_trend",
        "free_cash_flow_ttm",
        "pe_ratio",
        "pb_ratio",
    ] {
        assert!(holding[field].to_string().contains("null"), "{field}");
    }
    assert_eq!(
        holding["warnings"]["items"]["$ref"],
        "#/components/schemas/PortfolioWarningValue"
    );
    assert_eq!(
        schemas["PortfolioWarningValue"]["enum"],
        serde_json::json!([
            "revenue_decline",
            "margins_all_down",
            "operating_loss",
            "negative_free_cash_flow"
        ])
    );
    assert_eq!(
        schemas["PortfolioMarginTrendValue"]["enum"],
        serde_json::json!(["all_up", "all_down", "mixed"])
    );
    assert_eq!(response["warning_symbols"]["type"], "integer");
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
        "beta",
        "alpha_percent",
        "correlation",
        "tracking_error_percent",
        "volatility_percent",
        "benchmark_volatility_percent",
        "risk_days",
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

/// 持股穿透 path 的契約：member_id、limit 範圍與預設，ETF 前十大日期可為 null。
#[test]
fn openapi_portfolio_etf_exposure_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/portfolio/etf-exposure");
    assert_endpoint_responses(operation, "PortfolioEtfExposureResponse", false);
    let limit = query_schema(operation, "limit");
    assert_eq!(
        (
            limit["default"].clone(),
            limit["minimum"].clone(),
            limit["maximum"].clone()
        ),
        (20.into(), 1.into(), 200.into())
    );
    assert_eq!(query_schema(operation, "member_id")["minimum"], 0);
    let schemas = &document["components"]["schemas"];
    for field in [
        "total_value",
        "etf_value",
        "uncovered_value",
        "total_items",
        "etfs",
        "items",
        "unpriced",
    ] {
        assert!(
            schemas["PortfolioEtfExposureResponse"]["properties"]
                .get(field)
                .is_some(),
            "缺少 {field}"
        );
    }
    assert!(
        schemas["ExposureEtfItem"]["properties"]["holdings_as_of"]
            .to_string()
            .contains("null")
    );
    assert_eq!(
        schemas["ExposureItem"]["properties"]["sources"]["items"]["$ref"],
        "#/components/schemas/ExposureSourceItem"
    );
}

/// 持股再平衡與目標配置 path 的 OpenAPI 契約。
#[test]
fn openapi_portfolio_rebalance_and_targets_pins_schema() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");

    // 1. /api/v1/portfolio/rebalance
    let rebalance_op = get_operation(&document, "/api/v1/portfolio/rebalance");
    assert_endpoint_responses(rebalance_op, "PortfolioRebalanceResponse", false);

    let schemas = &document["components"]["schemas"];
    let rebalance_props = &schemas["PortfolioRebalanceResponse"]["properties"];
    assert_eq!(
        rebalance_props["stock_items"]["items"]["$ref"],
        "#/components/schemas/StockRebalanceItemDto"
    );
    assert_eq!(
        rebalance_props["industry_items"]["items"]["$ref"],
        "#/components/schemas/IndustryRebalanceItemDto"
    );
    assert_eq!(
        rebalance_props["etf_items"]["items"]["$ref"],
        "#/components/schemas/EtfLookThroughRebalanceItemDto"
    );
    assert_eq!(
        rebalance_props["cash_summary"]["$ref"],
        "#/components/schemas/RebalanceCashSummaryDto"
    );

    // 2. /api/v1/portfolio/targets
    let targets_get_op = get_operation(&document, "/api/v1/portfolio/targets");
    assert_endpoint_responses(targets_get_op, "PortfolioTargetsResponse", false);

    let targets_props = &schemas["PortfolioTargetsResponse"]["properties"];
    assert_eq!(
        targets_props["targets"]["items"]["$ref"],
        "#/components/schemas/PortfolioTargetItemDto"
    );
}
