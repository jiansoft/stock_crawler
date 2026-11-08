//! `/market/signals` handler：全市場技術訊號選股的測試。

use super::*;

fn row(symbol: &str, indicator: &str, kind: &str, trade_value: Option<i64>) -> SignalRow {
    SignalRow {
        stock_symbol: symbol.to_owned(),
        name: format!("{symbol}名稱"),
        market_id: if symbol.ends_with('T') { 4 } else { 2 },
        industry: "半導體業".to_owned(),
        closing_price: Some(Decimal::new(1234, 1)),
        change_range: Some(Decimal::new(-15, 1)),
        trade_value: trade_value.map(Decimal::from),
        indicator: indicator.to_owned(),
        kind: kind.to_owned(),
        description: format!("{symbol} {indicator} {kind}"),
    }
}

fn params(kind: Option<&str>, indicator: Option<&str>) -> MarketSignalsParams {
    MarketSignalsParams {
        date: None,
        market: None,
        kind: kind.map(str::to_owned),
        indicator: indicator.map(str::to_owned),
        industry: Some("半導體業".to_owned()),
        limit: None,
    }
}

fn rows() -> Vec<SignalRow> {
    vec![
        row("A", "kd", "overbought", Some(100)),
        row("B", "52_week", "new_high", Some(500)),
        row("CT", "kd", "golden_cross", None),
        row("C", "kd", "golden_cross", Some(500)),
        row("A", "kd", "golden_cross", Some(100)),
    ]
}

fn date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 12).expect("有效日期")
}

/// 家數依來源、種類的對外順序統計，不受篩選影響；項目依成交值排序並套用 limit。
#[test]
fn build_response_counts_filters_and_sorts() {
    let response = build_response(
        date(),
        "all",
        &params(Some("golden_cross"), None),
        2,
        rows(),
    );
    assert_eq!(response.data_as_of, "2026-10-12");
    assert_eq!(response.kind.as_deref(), Some("golden_cross"));
    assert_eq!(response.industry.as_deref(), Some("半導體業"));
    assert!(response.adjusted);
    let summary: Vec<(&str, &str, usize)> = response
        .summary
        .iter()
        .map(|c| (c.indicator.as_str(), c.kind.as_str(), c.count))
        .collect();
    assert_eq!(
        summary,
        [
            ("kd", "golden_cross", 3),
            ("kd", "overbought", 1),
            ("52_week", "new_high", 1)
        ]
    );
    assert_eq!(response.total, 3);
    let symbols: Vec<&str> = response
        .items
        .iter()
        .map(|item| item.stock_symbol.as_str())
        .collect();
    assert_eq!(symbols, ["C", "A"]);
    let first = &response.items[0];
    assert_eq!(first.market, "twse");
    assert_eq!(first.closing_price, Some(123.4));
    assert_eq!(first.change_percent, Some(-1.5));
    assert_eq!(first.trade_value, Some(500.0));
    assert_eq!(first.description, "C kd golden_cross");
}

/// 依來源篩選；沒有成交值的排在最後，上櫃股票標為 tpex。
#[test]
fn build_response_filters_by_indicator() {
    let response = build_response(date(), "all", &params(None, Some("kd")), 50, rows());
    let items: Vec<(&str, &str)> = response
        .items
        .iter()
        .map(|item| (item.stock_symbol.as_str(), item.kind.as_str()))
        .collect();
    assert_eq!(
        items,
        [
            ("C", "golden_cross"),
            ("A", "golden_cross"),
            ("A", "overbought"),
            ("CT", "golden_cross")
        ]
    );
    assert_eq!(response.items[3].market, "tpex");
    assert_eq!(response.items[3].trade_value, None);
}

/// 未知的來源或種類（資料表手動寫入的舊值）排在最後，不會讓排序失敗。
#[test]
fn unknown_names_sort_last() {
    assert_eq!(signal_indicator_order("unknown"), usize::MAX);
    assert_eq!(signal_kind_order("unknown"), usize::MAX);
    assert_eq!(signal_indicator_order("ma5_ma20"), 0);
    assert_eq!(signal_kind_order("new_low"), 7);
}
