//! OpenAPI 文件的契約測試：路徑、參數範圍、列舉值與回應欄位。

//! OpenAPI 產物的契約測試。
//!
//! 這些斷言全部從指定的 path 或 component 定位，不做全文搜尋，避免
//! 跨 path 的假陽性；契約一旦被改動就會在此立即失敗。
//!
//! 本檔放共用的定位工具與跨 path 的檢查；各 endpoint 的契約依領域分組放在子模組（`phases`、`market`、
//! `flows`、`fundamentals`、`technicals`、`portfolio`）。

use utoipa::OpenApi;

use super::ApiDoc;

mod flows;
mod fundamentals;
mod market;
mod phases;
mod portfolio;
mod technicals;

/// 取得指定 GET operation；path 缺漏時立即顯示精確路徑。
pub(super) fn get_operation<'a>(
    document: &'a serde_json::Value,
    path: &str,
) -> &'a serde_json::Value {
    document["paths"][path]["get"]
        .as_object()
        .map(|_| &document["paths"][path]["get"])
        .unwrap_or_else(|| panic!("OpenAPI 缺少 GET {path}"))
}

/// 驗證 operation 的 status code 指向指定 component schema。
pub(super) fn assert_response(operation: &serde_json::Value, status: &str, schema: &str) {
    let reference = operation["responses"][status]["content"]["application/json"]["schema"]["$ref"]
        .as_str()
        .unwrap_or_else(|| panic!("response {status} 缺少 JSON schema ref"));
    assert_eq!(reference, format!("#/components/schemas/{schema}"));
}

/// 取得 query parameter schema，避免以全文 contains 造成跨 path 假陽性。
pub(super) fn query_schema<'a>(
    operation: &'a serde_json::Value,
    name: &str,
) -> &'a serde_json::Value {
    operation["parameters"]
        .as_array()
        .expect("parameters 應為陣列")
        .iter()
        .find(|parameter| parameter["name"] == name && parameter["in"] == "query")
        .map(|parameter| &parameter["schema"])
        .unwrap_or_else(|| panic!("缺少 query parameter {name}"))
}

/// 解析 utoipa 可能 inline、`$ref` 或以 `allOf` 包裝的 enum schema。
pub(super) fn enum_values(
    document: &serde_json::Value,
    schema: &serde_json::Value,
) -> serde_json::Value {
    if schema["enum"].is_array() {
        return schema["enum"].clone();
    }
    if let Some(reference) = schema["$ref"].as_str() {
        let resolved = document
            .pointer(reference.trim_start_matches('#'))
            .expect("enum $ref 應指向有效 component");
        return enum_values(document, resolved);
    }
    if let Some(all_of) = schema["allOf"].as_array() {
        for nested in all_of {
            let values = enum_values(document, nested);
            if values.is_array() {
                return values;
            }
        }
    }
    if let Some(any_of) = schema["anyOf"].as_array() {
        for nested in any_of {
            let values = enum_values(document, nested);
            if values.is_array() {
                return values;
            }
        }
    }
    if let Some(one_of) = schema["oneOf"].as_array() {
        for nested in one_of {
            let values = enum_values(document, nested);
            if values.is_array() {
                return values;
            }
        }
    }
    panic!("無法解析 enum schema: {schema}")
}

/// 驗證成功與標準錯誤 responses；`has_not_found` 控制是否要求 404。
pub(super) fn assert_endpoint_responses(
    operation: &serde_json::Value,
    success_schema: &str,
    has_not_found: bool,
) {
    assert_response(operation, "200", success_schema);
    for status in ["401", "422", "500"] {
        assert_response(operation, status, "ErrorBody");
    }
    if has_not_found {
        assert_response(operation, "404", "ErrorBody");
    }
}

/// utoipa 6 對「inline 列舉＋default」會產生 `allOf: [列舉, {type: object, default}]`，
/// 要求參數同時是字串又是物件；有預設值的列舉參數因此改用 `$ref`（default 為同層欄位），
/// 這裡確保所有 query 參數都不再出現這種互相矛盾的 schema。
#[test]
fn query_parameters_have_no_contradictory_all_of() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let mut checked = 0;
    for (path, item) in document["paths"].as_object().expect("paths") {
        for parameter in item["get"]["parameters"].as_array().into_iter().flatten() {
            let schema = parameter["schema"].to_string();
            assert!(
                !(schema.contains("allOf") && schema.contains(r#""type":"object""#)),
                "{path} {} 的 schema 互相矛盾：{schema}",
                parameter["name"]
            );
            checked += 1;
        }
    }
    assert!(checked > 0);

    // 有預設值的列舉參數是 `$ref` 加上同層 default，列舉值由 components 提供。
    let rank_by = query_schema(get_operation(&document, "/api/v1/market/movers"), "rank_by");
    assert_eq!(rank_by["$ref"], "#/components/schemas/MoversRankByValue");
    assert_eq!(rank_by["default"], "top_gainers");
}

/// OpenAPI 契約須列出所有資料查詢路徑與健康檢查，供 Go client codegen 使用。
#[test]
fn openapi_contains_all_data_api_paths() {
    let json = ApiDoc::openapi()
        .to_json()
        .expect("OpenAPI should serialize");
    for path in [
        "/api/v1/stocks/search",
        "/api/v1/stocks/{symbol}/latest-quote",
        "/api/v1/stocks/{symbol}/price-history",
        "/api/v1/stocks/{symbol}/profile",
        "/api/v1/stocks/{symbol}/realtime-snapshot",
        "/api/v1/stocks/{symbol}/monthly-revenues",
        "/api/v1/stocks/{symbol}/financial-statements",
        "/api/v1/stocks/{symbol}/financial-reports",
        "/api/v1/stocks/{symbol}/financial-health",
        "/api/v1/stocks/{symbol}/dividends",
        "/api/v1/stocks/{symbol}/dividend-fill",
        "/api/v1/stocks/{symbol}/valuation",
        "/api/v1/stocks/{symbol}/chip",
        "/api/v1/market/breadth",
        "/api/v1/market/dividend-yield-ranking",
        "/api/v1/stocks/screen",
        "/api/v1/stocks/screen/backtest",
        "/api/v1/market/index-history",
        "/api/v1/market/dividend-calendar",
        "/api/v1/market/qfii-holding-ranking",
        "/api/v1/market/movers",
        "/api/v1/market/industries",
        "/api/v1/stocks/{symbol}/technicals",
        "/api/v1/market/signals",
        "/api/v1/market/relative-strength",
        "/api/v1/stocks/{symbol}/relative-strength",
        "/api/v1/market/industry-strength",
        "/api/v1/market/institutional-flow",
        "/api/v1/market/institutional-streak/performance",
        "/api/v1/market/industry-flow",
        "/api/v1/market/margin-ranking",
        "/api/v1/market/cagr-ranking",
        "/api/v1/market/cagr-ranking/{stock_symbol}",
        "/api/v1/healthz",
    ] {
        assert!(json.contains(path), "OpenAPI should contain {path}");
    }
    assert!(json.contains("bearer_auth"));
}
