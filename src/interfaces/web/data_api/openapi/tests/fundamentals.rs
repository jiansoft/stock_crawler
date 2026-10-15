//! 三大報表、財務體質、估值區間、同業比較、填息與股利現金流 path 的 OpenAPI 契約。

use utoipa::OpenApi;

use super::super::ApiDoc;
use super::{assert_endpoint_responses, enum_values, get_operation, query_schema};

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
