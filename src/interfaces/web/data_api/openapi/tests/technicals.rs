//! 技術指標、訊號選股、訊號績效與相對強度 path 的 OpenAPI 契約。

use utoipa::OpenApi;

use super::super::ApiDoc;
use super::{assert_endpoint_responses, get_operation, query_schema};

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
    for name in ["min_rs_score", "max_rs_score"] {
        let schema = query_schema(operation, name);
        assert_eq!(schema["minimum"], 1, "{name}");
        assert_eq!(schema["maximum"], 99, "{name}");
    }

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

/// 相對強度排行 path 的契約：參數範圍、列舉值與可為 null 的欄位。
#[test]
fn openapi_relative_strength_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/market/relative-strength");
    assert_endpoint_responses(operation, "RelativeStrengthResponse", true);
    let limit = query_schema(operation, "limit");
    assert_eq!(limit["default"], 50);
    assert_eq!(limit["minimum"], 1);
    assert_eq!(limit["maximum"], 200);
    assert!(
        query_schema(operation, "order")
            .to_string()
            .contains("SortOrderParamValue")
    );
    for name in ["date", "industry", "symbol"] {
        assert!(query_schema(operation, name).is_object(), "{name}");
    }

    let schemas = &document["components"]["schemas"];
    let item = &schemas["RelativeStrengthItem"]["properties"];
    for field in ["return_20d", "closing_price"] {
        assert!(item[field].to_string().contains("null"), "{field}");
    }
    assert_eq!(item["rs_score"]["type"], "integer");
    let response = &schemas["RelativeStrengthResponse"]["properties"];
    assert!(
        response["items"]
            .to_string()
            .contains("RelativeStrengthItem")
    );
}
