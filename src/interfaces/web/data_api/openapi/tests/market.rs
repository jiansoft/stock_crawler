//! 漲跌幅排行、CAGR 與產業表現 path 的 OpenAPI 契約。

use utoipa::OpenApi;

use super::super::ApiDoc;
use super::{assert_endpoint_responses, enum_values, get_operation, query_schema};

/// 漲跌幅／成交量排行的 OpenAPI 契約（movers 計畫 §4）。
///
/// 除了一般的參數與 response 形狀，這裡刻意固定兩件容易被後人「順手統一」
/// 而破壞語意的事：
/// 1. `rank_by` 只有三個值，**沒有** `top_trade_value`（即時快照沒有成交
///    金額，提供它會讓同一參數在兩個時段語意不同）。
/// 2. `is_realtime` 是布林且**不可為 null**——本 endpoint 是唯一可能回
///    `true` 的分析型 endpoint，其餘工具固定 `false`。
#[test]
fn openapi_movers_schema_pins_sources_and_units() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");

    let movers = get_operation(&document, "/api/v1/market/movers");
    // 這個 endpoint 在完全沒有資料時回 404（部署異常等級），因此帶
    // has_not_found = true。
    assert_endpoint_responses(movers, "MarketMoversResponse", true);

    let rank_by = query_schema(movers, "rank_by");
    assert_eq!(rank_by["default"], "top_gainers");
    assert_eq!(
        enum_values(&document, rank_by),
        serde_json::json!(["top_gainers", "top_losers", "top_volume"])
    );
    assert_eq!(
        enum_values(&document, query_schema(movers, "market")),
        serde_json::json!(["all", "twse", "tpex"])
    );
    let limit = query_schema(movers, "limit");
    assert_eq!(limit["default"], 20);
    assert_eq!(limit["minimum"], 1);
    assert_eq!(limit["maximum"], 50);

    let properties = &document["components"]["schemas"]["MarketMoversResponse"]["properties"];
    assert_eq!(properties["movers"]["type"], "array");
    assert_eq!(
        properties["movers"]["items"]["$ref"],
        "#/components/schemas/MarketMover"
    );
    // data_as_of 一定有值（即時為今日、收盤為該交易日），不可為 null。
    assert!(!properties["data_as_of"].to_string().contains("null"));
    assert_eq!(properties["is_realtime"]["type"], "boolean");
    // snapshot_updated_at 只有即時來源有值，必須是 nullable。
    assert!(
        properties["snapshot_updated_at"]
            .to_string()
            .contains("null")
    );

    // 兩種成交量單位是分開的欄位，且都可為 null——這是「不做張／股換算」
    // 這個決策在契約上的體現，不可被合併成單一欄位。
    let mover = &document["components"]["schemas"]["MarketMover"]["properties"];
    for field in [
        "volume_lots",
        "volume_shares",
        "trade_value",
        "transaction",
        "last_close",
        "source_site",
    ] {
        assert!(
            mover[field].to_string().contains("null"),
            "{field} 應為 nullable"
        );
    }
}

/// M4 每日 CAGR 兩條 path 的 OpenAPI 契約：responses、白名單 enum、
/// 預設值與陣列 item；並釘住「所有金額與比率為字串」這條前端契約。
#[test]
fn openapi_cagr_schemas_pin_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");

    let ranking = get_operation(&document, "/api/v1/market/cagr-ranking");
    assert_endpoint_responses(ranking, "CagrRankingResponse", true);
    let period = query_schema(ranking, "period");
    assert_eq!(period["default"], "Y1");
    assert_eq!(
        enum_values(&document, period),
        serde_json::json!(["M3", "M6", "Y1", "Y1H", "Y2", "Y3", "Y5", "Y7", "Y10"])
    );
    let metric = query_schema(ranking, "metric");
    assert_eq!(metric["default"], "total");
    assert_eq!(
        enum_values(&document, metric),
        serde_json::json!(["price", "total", "reinvested"])
    );
    assert_eq!(
        enum_values(&document, query_schema(ranking, "sort")),
        serde_json::json!(["cagr", "total_return"])
    );
    assert_eq!(query_schema(ranking, "market")["default"], "all");
    assert_eq!(query_schema(ranking, "stock_industry_id")["minimum"], 1);
    assert_eq!(query_schema(ranking, "include_incomplete")["default"], true);
    let limit = query_schema(ranking, "limit");
    assert_eq!(limit["default"], 50);
    assert_eq!(limit["minimum"], 1);
    assert_eq!(limit["maximum"], 200);
    let offset = query_schema(ranking, "offset");
    assert_eq!(offset["default"], 0);
    assert_eq!(offset["minimum"], 0);

    let properties = &document["components"]["schemas"]["CagrRankingResponse"]["properties"];
    assert_eq!(properties["items"]["type"], "array");
    assert_eq!(
        properties["items"]["items"]["$ref"],
        "#/components/schemas/CagrRankingItem"
    );
    // principal 與各種計數是整數，維持 JSON number。
    assert_eq!(properties["principal"]["type"], "integer");
    assert_eq!(properties["total"]["type"], "integer");
    // base_date／years 在整頁皆資料不足時為 null。
    assert!(properties["base_date"].to_string().contains("null"));
    assert!(properties["years"].to_string().contains("null"));

    // §M4 契約核心：Decimal 一律字串，計數一律整數。
    let coverage = &document["components"]["schemas"]["CagrCoverageInfo"]["properties"];
    assert_eq!(coverage["coverage_ratio"]["type"], "string");
    assert_eq!(coverage["universe"]["type"], "integer");
    assert_eq!(coverage["counted"]["type"], "integer");
    assert_eq!(coverage["survivorship_note"]["type"], "boolean");
    let summary = &document["components"]["schemas"]["CagrSummary"]["properties"];
    assert_eq!(summary["positive"]["type"], "integer");
    assert_eq!(summary["positive_ratio"]["type"], "string");

    let item = &document["components"]["schemas"]["CagrRankingItem"]["properties"];
    for field in [
        "base_price",
        "end_price",
        "end_shares",
        "cash_received",
        "end_value",
        "total_return_pct",
        "cagr_pct",
    ] {
        let schema = item[field].to_string();
        assert!(schema.contains("string"), "{field} 必須序列化為字串");
        assert!(schema.contains("null"), "{field} 資料不足時必須可為 null");
    }
    assert!(item["rank"].to_string().contains("null"));
    assert_eq!(item["dividend_events"]["type"], "integer");
    assert_eq!(item["data_complete"]["type"], "boolean");
    assert_eq!(item["stock_industry_id"]["type"], "integer");

    // 個股端點：items 是 CagrPeriodItem（多 period、無 rank）。
    let symbol = get_operation(&document, "/api/v1/market/cagr-ranking/{stock_symbol}");
    assert_endpoint_responses(symbol, "CagrSymbolResponse", true);
    assert_eq!(query_schema(symbol, "metric")["default"], "total");
    let symbol_properties = &document["components"]["schemas"]["CagrSymbolResponse"]["properties"];
    assert_eq!(symbol_properties["items"]["type"], "array");
    assert_eq!(
        symbol_properties["items"]["items"]["$ref"],
        "#/components/schemas/CagrPeriodItem"
    );
    let period_item = &document["components"]["schemas"]["CagrPeriodItem"]["properties"];
    assert_eq!(period_item["period"]["type"], "string");
    assert!(period_item["years"].to_string().contains("string"));
    assert!(
        period_item.get("rank").is_none(),
        "個股端點的項目不應有 rank"
    );
}

/// 產業表現 path 的契約：days 範圍與預設、market 列舉、可為 null 的統計欄位。
#[test]
fn openapi_industries_schema_pins_field_names() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI 可序列化");
    let operation = get_operation(&document, "/api/v1/market/industries");
    assert_endpoint_responses(operation, "IndustryPerformanceResponse", true);
    let days = query_schema(operation, "days");
    assert_eq!(days["default"], 1);
    assert_eq!(days["minimum"], 1);
    assert_eq!(days["maximum"], 60);
    assert_eq!(
        query_schema(operation, "market")["$ref"],
        "#/components/schemas/MarketParamValue"
    );

    let schemas = &document["components"]["schemas"];
    let response = &schemas["IndustryPerformanceResponse"]["properties"];
    assert!(response["base_date"].to_string().contains("null"));
    assert_eq!(
        response["industries"]["items"]["$ref"],
        "#/components/schemas/IndustryPerformance"
    );
    let item = &schemas["IndustryPerformance"]["properties"];
    for field in [
        "median_change_percent",
        "cap_weighted_change_percent",
        "trade_value_share_percent",
        "median_price_earning_ratio",
        "top_gainer",
        "top_loser",
    ] {
        assert!(item[field].to_string().contains("null"), "{field}");
    }
}
