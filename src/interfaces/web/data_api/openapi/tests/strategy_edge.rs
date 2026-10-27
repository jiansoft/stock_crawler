//! 策略優勢快取端點的固定 schema 與錯誤契約。

use super::*;

/// 對外欄位、nullable 期間及判斷列舉固定。
#[test]
fn strategy_edge_contract() {
    let document = serde_json::to_value(ApiDoc::openapi()).unwrap();
    let operation = get_operation(&document, "/api/v1/strategy-presets/edge");
    assert_response(operation, "200", "StrategyEdgeResponse");
    for status in ["401", "404", "500", "503"] {
        assert_response(operation, status, "ErrorBody");
    }
    let schemas = &document["components"]["schemas"];
    assert_eq!(
        schemas["StrategyEdgeVerdict"]["enum"],
        serde_json::json!(["holds", "weakened", "unknown"])
    );
    for (name, fields) in [
        (
            "StrategyEdgeResponse",
            vec![
                "strategies",
                "execution_price",
                "slippage_bps",
                "disclaimer",
            ],
        ),
        (
            "StrategyEdgeItem",
            vec!["name", "checked", "recent", "full", "verdict"],
        ),
        (
            "StrategyEdgeWindow",
            vec![
                "from",
                "to",
                "offsets",
                "median_return",
                "etf_median_return",
                "beat_rate",
            ],
        ),
    ] {
        let properties = schemas[name]["properties"].as_object().unwrap();
        assert_eq!(properties.len(), fields.len());
        for field in fields {
            assert!(properties.contains_key(field), "{name}.{field}");
        }
    }
}
