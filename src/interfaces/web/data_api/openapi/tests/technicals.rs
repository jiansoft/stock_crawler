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
        ("min_avg_trade_value", 100000),
        ("max_weight", 100),
    ] {
        let schema = query_schema(operation, name);
        assert_eq!(schema["minimum"], 1, "{name}");
        assert_eq!(schema["maximum"], maximum, "{name}");
    }
    let etf_weight = query_schema(operation, "etf_weight");
    assert_eq!(
        (
            etf_weight["minimum"].as_u64(),
            etf_weight["maximum"].as_u64()
        ),
        (Some(0), Some(100))
    );
    assert_eq!(etf_weight["default"], 0);
    let rank_by = query_schema(operation, "rank_by");
    assert_eq!(
        rank_by["enum"],
        serde_json::json!(["rs_score", "trade_value", "foreign_streak", "trust_streak"])
    );
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
    assert_eq!(current["cash"]["type"], "number");
    for field in ["return_to_date", "etf_return_to_date"] {
        assert!(current[field].to_string().contains("null"), "{field}");
    }
    assert!(
        schemas["BacktestHolding"]["properties"]["return_to_date"]
            .to_string()
            .contains("null")
    );
    assert!(
        query_schema(operation, "exclude_industry")
            .to_string()
            .contains("string")
    );
    assert!(response["exclude_industry"].to_string().contains("null"));
    assert!(response["max_weight"].to_string().contains("null"));
    for name in ["stop_loss", "trailing_stop"] {
        let schema = query_schema(operation, name);
        assert_eq!(
            (schema["minimum"].as_u64(), schema["maximum"].as_u64()),
            (Some(1), Some(50)),
            "{name}"
        );
        assert!(response[name].to_string().contains("null"), "{name}");
    }
    assert_eq!(
        schemas["BacktestPeriod"]["properties"]["stopped"]["type"],
        "integer"
    );
    assert_eq!(
        schemas["BacktestSummary"]["properties"]["stop_exits"]["type"],
        "integer"
    );
    assert!(current["holdings"].to_string().contains("BacktestHolding"));
    assert!(current["exited"].to_string().contains("BacktestExit"));
    assert_eq!(
        schemas["BacktestHolding"]["properties"]["added"]["type"],
        "boolean"
    );
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

/// 換股日偏移穩健度 path 的契約：與單次回測共用參數、回應含各偏移彙總與分布。
#[test]
fn openapi_screen_backtest_offsets_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/stocks/screen/backtest/offsets");
    assert_endpoint_responses(operation, "ScreenBacktestOffsetsResponse", true);
    assert_eq!(query_schema(operation, "hold_days")["default"], 20);
    assert_eq!(
        query_schema(operation, "min_avg_trade_value")["maximum"],
        100000
    );

    let schemas = &document["components"]["schemas"];
    let response = &schemas["ScreenBacktestOffsetsResponse"]["properties"];
    assert_eq!(response["hold_days"]["type"], "integer");
    assert_eq!(response["offsets"]["type"], "array");
    assert_eq!(response["years"]["type"], "array");
    assert_eq!(response["rank_by"]["type"], "string");
    for field in ["split", "before_split", "after_split"] {
        assert!(response[field].to_string().contains("null"), "{field}");
    }
    assert!(
        response["after_split"]
            .to_string()
            .contains("BacktestOffsetDistribution")
    );
    assert!(
        query_schema(operation, "split")
            .to_string()
            .contains("string")
    );
    let year = &schemas["BacktestOffsetYear"]["properties"];
    for field in [
        "year",
        "offsets",
        "return_after_costs",
        "etf_return",
        "beat_etf_rate",
    ] {
        assert!(year.get(field).is_some(), "{field}");
    }
    assert!(
        response["offsets"]
            .to_string()
            .contains("BacktestOffsetResult")
    );
    assert!(
        response["distribution"]
            .to_string()
            .contains("BacktestOffsetDistribution")
    );
    let result = &schemas["BacktestOffsetResult"]["properties"];
    assert_eq!(result["rebalance_offset"]["type"], "integer");
    assert!(result["summary"].to_string().contains("BacktestSummary"));
    let spread = &schemas["BacktestOffsetDistribution"]["properties"];
    for field in [
        "annualized_return_after_costs",
        "sharpe_ratio",
        "max_drawdown_after_costs",
        "etf_annualized_return",
        "etf_sharpe_ratio",
        "volatility",
        "etf_volatility",
        "etf_max_drawdown",
    ] {
        assert!(
            spread[field].to_string().contains("BacktestStat"),
            "{field}"
        );
        assert!(spread[field].to_string().contains("null"), "{field}");
    }
    assert!(spread["beat_etf_return_rate"].to_string().contains("null"));
    assert!(
        spread["beat_etf_drawdown_rate"]
            .to_string()
            .contains("null")
    );
    let stat = &schemas["BacktestStat"]["properties"];
    for field in ["min", "p25", "median", "mean", "p75", "max"] {
        assert_eq!(stat[field]["type"], "number", "{field}");
    }
}

/// 0050 核心部位前緣 path 的契約：與單次回測共用參數、回應含各比例的分布與最佳比例。
#[test]
fn openapi_screen_backtest_frontier_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/stocks/screen/backtest/frontier");
    assert_endpoint_responses(operation, "ScreenBacktestFrontierResponse", true);
    assert_eq!(query_schema(operation, "hold_days")["default"], 20);

    let schemas = &document["components"]["schemas"];
    let response = &schemas["ScreenBacktestFrontierResponse"]["properties"];
    assert_eq!(response["offsets"]["type"], "integer");
    assert_eq!(response["weights"]["type"], "array");
    assert!(
        response["weights"]
            .to_string()
            .contains("BacktestFrontierPoint")
    );
    for field in [
        "best_sharpe_weight",
        "best_return_weight",
        "split",
        "best_sharpe_weight_before_split",
    ] {
        assert!(response[field].to_string().contains("null"), "{field}");
    }
    let point = &schemas["BacktestFrontierPoint"]["properties"];
    for field in ["before_split", "after_split"] {
        assert!(point[field].to_string().contains("null"), "{field}");
    }
    assert_eq!(point["etf_weight"]["type"], "integer");
    assert!(
        point["distribution"]
            .to_string()
            .contains("BacktestOffsetDistribution")
    );
}

/// 報酬歸因 path 的契約：與單次回測共用參數、回應含整段／年度合計與產業效果。
#[test]
fn openapi_screen_backtest_attribution_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/stocks/screen/backtest/attribution");
    assert_endpoint_responses(operation, "ScreenBacktestAttributionResponse", true);
    assert_eq!(query_schema(operation, "hold_days")["default"], 20);

    let schemas = &document["components"]["schemas"];
    let response = &schemas["ScreenBacktestAttributionResponse"]["properties"];
    assert_eq!(response["benchmark_size"]["type"], "integer");
    assert!(
        response["summary"]
            .to_string()
            .contains("BacktestAttributionTotals")
    );
    assert!(
        response["years"]
            .to_string()
            .contains("BacktestAttributionYear")
    );
    assert!(
        response["industries"]
            .to_string()
            .contains("BacktestIndustryEffect")
    );
    let totals = &schemas["BacktestAttributionTotals"]["properties"];
    for field in [
        "strategy_return",
        "benchmark_return",
        "excess_return",
        "allocation",
        "selection",
        "interaction",
    ] {
        assert_eq!(totals[field]["type"], "number", "{field}");
    }
    assert!(totals["etf_return"].to_string().contains("null"));
    let effect = &schemas["BacktestIndustryEffect"]["properties"];
    assert_eq!(effect["industry"]["type"], "string");
    assert_eq!(effect["total"]["type"], "number");
}
