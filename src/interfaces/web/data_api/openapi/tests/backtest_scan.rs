//! 條件選股回測的偏移穩健度、0050 前緣、個股歸因與滾動前推 path 的 OpenAPI 契約。

use super::super::ApiDoc;
use super::{assert_endpoint_responses, get_operation, query_schema};
use utoipa::OpenApi;

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

/// 滾動前推 path 的契約：共用回測參數加 `train_years`，回應含逐年參數、彙總與各偏移分布。
#[test]
fn openapi_screen_backtest_walk_forward_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/stocks/screen/backtest/walk-forward");
    assert_endpoint_responses(operation, "ScreenBacktestWalkForwardResponse", true);
    let train_years = query_schema(operation, "train_years");
    assert_eq!(
        (
            train_years["minimum"].as_u64(),
            train_years["maximum"].as_u64(),
            train_years["default"].as_u64()
        ),
        (Some(1), Some(5), Some(3))
    );

    let schemas = &document["components"]["schemas"];
    let response = &schemas["ScreenBacktestWalkForwardResponse"]["properties"];
    for field in ["train_years", "candidates", "rebalance_offset"] {
        assert_eq!(response[field]["type"], "integer", "{field}");
    }
    for (field, schema) in [
        ("summary", "BacktestSummary"),
        ("baseline", "BacktestSummary"),
        ("offsets", "BacktestOffsetDistribution"),
        ("baseline_offsets", "BacktestOffsetDistribution"),
        ("years", "BacktestWalkForwardYear"),
    ] {
        assert!(response[field].to_string().contains(schema), "{field}");
    }
    let year = &schemas["BacktestWalkForwardYear"]["properties"];
    for field in ["year", "etf_weight", "keep_rank", "periods"] {
        assert_eq!(year[field]["type"], "integer", "{field}");
    }
    for field in ["max_weight", "etf_return"] {
        assert!(year[field].to_string().contains("null"), "{field}");
    }
    assert_eq!(year["train_sharpe_ratio"]["type"], "number");
    assert_eq!(year["baseline_return_after_costs"]["type"], "number");
}
