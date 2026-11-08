//! `/market/institutional-flow` handler：三大法人買賣超排行的測試。

use super::*;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("合法日期")
}

fn row(symbol: &str, market_id: i32, industry: &str, net_shares: i64, amount: f64) -> FlowRow {
    FlowRow {
        stock_symbol: symbol.to_owned(),
        name: format!("{symbol}名稱"),
        market_id,
        industry: industry.to_owned(),
        net_shares,
        net_amount: amount,
        volume: Some(100_000.0),
        trade_value: Some(5_000_000.0),
        buy_days: 2,
        sell_days: 1,
        streak: 2,
        closing_price: Some(50.0),
    }
}

fn filter<'a>(side: &'a str, market_id: i32, industry: Option<&'a str>) -> FlowFilter<'a> {
    FlowFilter {
        investor: "trust",
        side,
        market: if market_id == 4 { "tpex" } else { "all" },
        market_id,
        industry,
        limit: 2,
    }
}

fn period() -> FlowPeriod {
    FlowPeriod {
        data_date: day(2026, 10, 13),
        from: day(2026, 10, 7),
        days: 3,
    }
}

fn universe() -> Vec<FlowRow> {
    vec![
        row("A", 2, "半導體業", 12_345, 600_000.4),
        row("B", 4, "半導體業", 8_000, 900_000.0),
        row("C", 2, "航運業", -20_600, -1_000_000.0),
        row("D", 2, "半導體業", 5_000, 600_000.4),
        row("E", 4, "航運業", -500, -20_000.0),
        row("F", 2, "航運業", 0, 0.0),
    ]
}

/// 買超依金額由大到小、同金額依代號；limit 截斷但 matched 不變；張數、金額與占比的換算。
#[test]
fn build_response_ranks_buyers() {
    let response = build_response(period(), filter("buy", 0, None), universe());
    assert_eq!(response.data_as_of, "2026-10-13");
    assert_eq!(response.from, "2026-10-07");
    assert_eq!(response.days, 3);
    assert_eq!(response.investor, "trust");
    assert_eq!(response.side, "buy");
    assert_eq!(response.market, "all");
    assert!(response.industry.is_none());
    assert_eq!(response.matched, 3, "金額為 0 的 F 不算買超");
    let ranked: Vec<(usize, &str)> = response
        .items
        .iter()
        .map(|item| (item.rank, item.stock_symbol.as_str()))
        .collect();
    assert_eq!(ranked, [(1, "B"), (2, "A")]);
    let a = &response.items[1];
    assert_eq!(a.market, "twse");
    assert_eq!(a.name, "A名稱");
    assert_eq!(a.net_shares, 12_345);
    assert_eq!(a.net_lots, 12);
    assert_eq!(a.net_amount, 600_000.0);
    assert_eq!(a.volume_ratio, Some(12.35));
    assert_eq!((a.buy_days, a.sell_days, a.streak), (2, 1, 2));
    assert_eq!(a.closing_price, Some(50.0));
    assert_eq!(response.items[0].market, "tpex");
}

/// 賣超依淨賣金額由大到小；市場與產業篩選；沒有成交量時占比為 null。
#[test]
fn build_response_ranks_sellers_with_filters() {
    let mut rows = universe();
    rows[2].volume = Some(0.0);
    let sellers = build_response(period(), filter("sell", 0, Some("航運業")), rows);
    assert_eq!(sellers.industry.as_deref(), Some("航運業"));
    let ranked: Vec<&str> = sellers
        .items
        .iter()
        .map(|item| item.stock_symbol.as_str())
        .collect();
    assert_eq!(ranked, ["C", "E"]);
    assert_eq!(sellers.items[0].net_lots, -21);
    assert_eq!(sellers.items[0].volume_ratio, None);

    let otc = build_response(period(), filter("sell", 4, None), universe());
    assert_eq!(otc.market, "tpex");
    assert_eq!(otc.matched, 1);
    assert_eq!(otc.items[0].stock_symbol, "E");
    assert_eq!(otc.items[0].volume_ratio, Some(-0.5));
}
