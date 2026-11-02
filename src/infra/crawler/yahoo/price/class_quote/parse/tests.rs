use rust_decimal_macros::dec;
use serde_json::json;

use super::*;

fn parse_test_item(value: serde_json::Value) -> Result<Option<(String, RealtimeSnapshot)>> {
    let raw = serde_json::to_vec(&value).unwrap();
    let item: ClassQuoteItem<'_> = serde_json::from_slice(&raw).unwrap();
    parse_class_quote_item(&item)
}

/// 驗證類股 API URL 會使用 Yahoo 實際可用的分號參數格式。
/// 有漲跌停價時記成區間；Yahoo 用 `-` 表示沒有漲跌幅限制（00715L）；缺欄位為未知。
#[test]
fn parse_class_quote_item_reads_price_limits() {
    let (_, ranged) = parse_test_item(json!({
        "systexId": "00631L",
        "price": {"raw": "39.58"},
        "limitUpPrice": {"raw": "47.54"},
        "limitDownPrice": {"raw": "31.7"}
    }))
    .unwrap()
    .unwrap();
    assert_eq!(
        ranged.price_limit,
        PriceLimit::Range {
            down: dec!(31.7),
            up: dec!(47.54)
        }
    );

    let (_, unlimited) = parse_test_item(json!({
        "systexId": "00715L",
        "price": {"raw": "70.75"},
        "limitUpPrice": {"raw": "-"},
        "limitDownPrice": {"raw": "-"}
    }))
    .unwrap()
    .unwrap();
    assert_eq!(unlimited.price_limit, PriceLimit::Unlimited);

    let (_, unknown) = parse_test_item(json!({"systexId": "2330", "price": {"raw": "2500"}}))
        .unwrap()
        .unwrap();
    assert_eq!(unknown.price_limit, PriceLimit::Unknown);
}

/// 驗證類股 JSON 會優先使用 `systexId` 當成股票代號，並正確解析各欄位。
#[test]
fn test_parse_class_quote_item_uses_systex_id_and_volume_k() {
    let item = json!({
        "symbol": "2330.TW",
        "symbolName": "台積電",
        "systexId": "2330",
        "price": { "raw": "998" },
        "change": { "raw": "-12" },
        "changePercent": "-1.19%",
        "regularMarketOpen": { "raw": "1005" },
        "regularMarketDayHigh": { "raw": "1010" },
        "regularMarketDayLow": { "raw": "995" },
        "regularMarketPreviousClose": { "raw": "1010" },
        "volumeK": 43210
    });

    let (symbol, snapshot) = parse_test_item(item).unwrap().unwrap();

    assert_eq!(symbol, "2330");
    assert_eq!(snapshot.name, "台積電");
    assert_eq!(snapshot.source_site, "Yahoo");
    assert_eq!(snapshot.price, dec!(998));
    assert_eq!(snapshot.change, dec!(-12));
    assert_eq!(snapshot.change_range, dec!(-1.19));
    assert_eq!(snapshot.open, dec!(1005));
    assert_eq!(snapshot.high, dec!(1010));
    assert_eq!(snapshot.low, dec!(995));
    assert_eq!(snapshot.last_close, dec!(1010));
    assert_eq!(snapshot.volume, dec!(43210));
}

/// 驗證當 `systexId` 缺失時，仍可由 `symbol` 去掉市場尾碼後取得股票代號。
#[test]
fn test_parse_class_quote_item_falls_back_to_symbol_without_suffix() {
    let item = json!({
        "symbol": "006208.TW",
        "symbolName": "富邦台50",
        "price": { "raw": "88.4" }
    });

    let (symbol, snapshot) = parse_test_item(item).unwrap().unwrap();

    assert_eq!(symbol, "006208");
    assert_eq!(snapshot.source_site, "Yahoo");
    assert_eq!(snapshot.price, dec!(88.4));
    assert_eq!(snapshot.volume, Decimal::ZERO);
}

#[test]
fn parse_class_quote_item_skips_rows_without_symbol() {
    let item = json!({
        "symbolName": "缺代號",
        "price": { "raw": "88.4" }
    });

    assert!(parse_test_item(item).unwrap().is_none());
}

#[test]
fn parse_class_quote_item_normalizes_missing_market_price_and_percent_text() {
    let item = json!({
        "symbol": "2317.TW",
        "symbolName": "鴻海",
        "price": { "raw": "市價" },
        "change": { "raw": "--" },
        "changePercent": "12.34%",
        "regularMarketOpen": { "raw": "-" },
        "volumeK": "1,234"
    });

    let (symbol, snapshot) = parse_test_item(item).unwrap().unwrap();

    assert_eq!(symbol, "2317");
    assert_eq!(snapshot.price, Decimal::ZERO);
    assert_eq!(snapshot.change, Decimal::ZERO);
    assert_eq!(snapshot.change_range, dec!(12.34));
    assert_eq!(snapshot.open, Decimal::ZERO);
    assert_eq!(snapshot.volume, dec!(1234));
}

#[test]
fn parse_class_quote_item_reports_malformed_decimal() {
    let item = json!({
        "symbol": "2330.TW",
        "price": { "raw": "not-a-number" }
    });

    let error = parse_test_item(item).unwrap_err().to_string();

    assert!(error.contains("Failed to parse Yahoo price for 2330"));
}

/// 驗證 Yahoo 的 `nextOffset` 欄位能被正確轉成數值型態。
#[test]
fn test_next_offset_parsing() {
    let pagination = ClassQuotesPagination {
        results_total: 89,
        next_offset: Some("60"),
    };

    assert_eq!(pagination.next_offset().unwrap(), Some(60));
}

#[test]
fn next_offset_reports_malformed_values() {
    let pagination = ClassQuotesPagination {
        results_total: 89,
        next_offset: Some("next-page"),
    };

    let error = pagination.next_offset().unwrap_err().to_string();

    assert!(error.contains("Failed to parse Yahoo class quote nextOffset"));
}
