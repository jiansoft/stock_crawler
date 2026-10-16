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
    for indicator in [
        "ma5_ma20",
        "close_ma240",
        "rsi14",
        "52_week",
        "rs80",
        "rs50",
    ] {
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
    for (name, maximum) in [
        ("min_rs_score", 99),
        ("max_rs_score", 99),
        ("min_foreign_streak", 60),
        ("min_trust_streak", 60),
    ] {
        let schema = query_schema(operation, name);
        assert_eq!(schema["minimum"], 1, "{name}");
        assert_eq!(schema["maximum"], maximum, "{name}");
    }

    assert!(
        query_schema(operation, "group_by")
            .to_string()
            .contains("GroupByParamValue")
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
    assert!(response["group_by"].to_string().contains("null"));
    assert!(
        response["benchmark_by_year"]
            .to_string()
            .contains("BenchmarkYear")
    );
    let years = schemas["SignalPerformanceItem"]["properties"]["years"].to_string();
    assert!(
        years.contains("PerformanceYear") && years.contains("null"),
        "{years}"
    );
    assert_eq!(
        schemas["GroupByParamValue"]["enum"],
        serde_json::json!(["year"])
    );
    let year = &schemas["PerformanceYear"]["properties"];
    assert_eq!(year["year"]["type"], "integer");
    assert!(year["horizons"].to_string().contains("HorizonStat"));
    assert!(
        schemas["BenchmarkYear"]["properties"]["benchmark"]
            .to_string()
            .contains("BenchmarkStat")
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

/// 條件選股回測 path 的契約：持有天數與檔數的預設、條件參數範圍、回應欄位。
#[test]
fn openapi_screen_backtest_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/stocks/screen/backtest");
    assert_endpoint_responses(operation, "ScreenBacktestResponse", true);
    assert_eq!(query_schema(operation, "hold_days")["default"], 20);
    let limit = query_schema(operation, "limit");
    assert_eq!(limit["default"], 20);
    assert_eq!(limit["minimum"], 1);
    assert_eq!(limit["maximum"], 100);
    let fee_discount = query_schema(operation, "fee_discount");
    assert_eq!(fee_discount["default"], 1.0);
    assert_eq!(fee_discount["minimum"], 0.0);
    assert_eq!(fee_discount["maximum"], 1.0);
    let offset = query_schema(operation, "rebalance_offset");
    assert_eq!(
        (offset["minimum"].as_u64(), offset["maximum"].as_u64()),
        (Some(0), Some(239))
    );
    for (name, maximum) in [
        ("min_rs_score", 99),
        ("max_rs_score", 99),
        ("min_foreign_streak", 60),
        ("min_trust_streak", 60),
    ] {
        let schema = query_schema(operation, name);
        assert_eq!(schema["minimum"], 1, "{name}");
        assert_eq!(schema["maximum"], maximum, "{name}");
    }
    for (name, schema) in [
        ("indicator", "SignalIndicatorValue"),
        ("kind", "SignalKindValue"),
        ("market", "MarketParamValue"),
    ] {
        assert!(
            query_schema(operation, name).to_string().contains(schema),
            "{name}"
        );
    }

    let schemas = &document["components"]["schemas"];
    let response = &schemas["ScreenBacktestResponse"]["properties"];
    assert_eq!(response["adjusted"]["type"], "boolean");
    assert_eq!(response["fee_discount"]["type"], "number");
    assert!(response["summary"].to_string().contains("BacktestSummary"));
    assert!(response["periods"].to_string().contains("BacktestPeriod"));
    assert!(response["years"].to_string().contains("BacktestYear"));
    assert!(
        response["contributors"]
            .to_string()
            .contains("BacktestContributors")
    );
    let contributor = &schemas["BacktestContributor"]["properties"];
    assert_eq!(contributor["contribution"]["type"], "number");
    assert!(contributor["name"].to_string().contains("null"));
    assert_eq!(
        schemas["BacktestContributors"]["properties"]["best"]["type"],
        "array"
    );
    assert!(response["current"].to_string().contains("BacktestCurrent"));
    assert!(response["current"].to_string().contains("null"));
    let current = &schemas["BacktestCurrent"]["properties"];
    assert_eq!(current["days_to_next_rebalance"]["type"], "integer");
    assert!(current["holdings"].to_string().contains("BacktestHolding"));
    assert_eq!(response["etf_benchmark"]["type"], "string");
    assert_eq!(response["keep_rank"]["type"], "integer");
    assert!(response["market_filter_ma"].to_string().contains("null"));
    assert!(response["hold_min_rs_score"].to_string().contains("null"));
    assert_eq!(
        schemas["BacktestPeriod"]["properties"]["market_filtered"]["type"],
        "boolean"
    );
    let year = &schemas["BacktestYear"]["properties"];
    assert_eq!(year["year"]["type"], "integer");
    assert!(year["etf_total_return"].to_string().contains("null"));
    let period = &schemas["BacktestPeriod"]["properties"];
    assert!(period["avg_return"].to_string().contains("null"));
    assert_eq!(period["symbols"]["type"], "array");
    assert_eq!(period["return_after_costs"]["type"], "number");
    assert_eq!(period["turnover"]["type"], "number");
    assert!(period["etf_return"].to_string().contains("null"));
    let summary = &schemas["BacktestSummary"]["properties"];
    for field in [
        "total_return",
        "annualized_return",
        "max_drawdown",
        "beat_market_rate",
        "avg_turnover",
        "total_return_after_costs",
        "annualized_return_after_costs",
        "max_drawdown_after_costs",
        "etf_total_return",
        "etf_annualized_return",
        "etf_max_drawdown",
        "beat_etf_rate",
        "volatility",
        "sharpe_ratio",
        "benchmark_volatility",
        "benchmark_sharpe_ratio",
        "etf_volatility",
        "etf_sharpe_ratio",
    ] {
        assert!(summary[field].to_string().contains("null"), "{field}");
    }
}
