//! `/market/industry-flow` handler：三大法人資金在各產業的淨流入流出的測試。

use chrono::NaiveDate;

use super::*;

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).expect("合法日期")
}

fn row(symbol: &str, market_id: i32, industry: &str, amount: f64) -> FlowRow {
    FlowRow {
        stock_symbol: symbol.to_owned(),
        name: format!("{symbol}名稱"),
        market_id,
        industry: industry.to_owned(),
        net_shares: 0,
        net_amount: amount,
        volume: None,
        trade_value: Some(1_000_000.0),
        buy_days: 0,
        sell_days: 0,
        streak: 0,
        closing_price: None,
    }
}

fn period() -> FlowPeriod {
    FlowPeriod {
        data_date: day(13),
        from: day(7),
        days: 5,
    }
}

fn universe() -> Vec<FlowRow> {
    vec![
        row("A", 2, "半導體業", 300_000.0),
        row("B", 4, "半導體業", -50_000.0),
        row("C", 2, "半導體業", 100_000.0),
        row("D", 2, "半導體業", 100_000.0),
        row("E", 2, "半導體業", 20_000.0),
        row("F", 2, "航運業", -200_000.0),
        row("G", 4, "航運業", -100_000.4),
        row("H", 2, "", 7_000.0),
    ]
}

/// 產業加總、家數、占比、買賣超個股（各最多 3 檔、同金額依代號）、未分類只計入合計、名次。
#[test]
fn build_response_aggregates_industries() {
    let response = build_response(period(), "trust", "all", 0, "desc", universe());
    assert_eq!(response.data_as_of, "2026-10-13");
    assert_eq!(response.from, "2026-10-07");
    assert_eq!(response.days, 5);
    assert_eq!(response.investor, "trust");
    assert_eq!(response.order, "desc");
    assert_eq!(response.total_net_amount, 177_000.0);
    let names: Vec<(usize, &str)> = response
        .items
        .iter()
        .map(|item| (item.rank, item.industry.as_str()))
        .collect();
    assert_eq!(names, [(1, "半導體業"), (2, "航運業")]);

    let semi = &response.items[0];
    assert_eq!((semi.stocks, semi.buy_stocks, semi.sell_stocks), (5, 4, 1));
    assert_eq!(semi.net_amount, 470_000.0);
    assert_eq!(semi.trade_value_ratio, Some(9.4));
    let buyers: Vec<&str> = semi
        .top_buyers
        .iter()
        .map(|stock| stock.stock_symbol.as_str())
        .collect();
    assert_eq!(buyers, ["A", "C", "D"]);
    assert_eq!(semi.top_buyers[0].name, "A名稱");
    assert_eq!(semi.top_sellers.len(), 1);
    assert_eq!(semi.top_sellers[0].net_amount, -50_000.0);

    let shipping = &response.items[1];
    assert_eq!(shipping.net_amount, -300_000.0);
    assert!(shipping.top_buyers.is_empty());
    let sellers: Vec<(&str, f64)> = shipping
        .top_sellers
        .iter()
        .map(|stock| (stock.stock_symbol.as_str(), stock.net_amount))
        .collect();
    assert_eq!(sellers, [("F", -200_000.0), ("G", -100_000.0)]);
}

/// 市場篩選、asc 反轉（名次不變）、沒有成交金額時占比為 null。
#[test]
fn build_response_filters_market_and_reverses() {
    let mut rows = universe();
    for row in &mut rows {
        row.trade_value = None;
    }
    let response = build_response(period(), "foreign", "tpex", 4, "asc", rows);
    assert_eq!(response.market, "tpex");
    assert_eq!(response.total_net_amount, -150_000.0);
    let names: Vec<(usize, &str, f64)> = response
        .items
        .iter()
        .map(|item| (item.rank, item.industry.as_str(), item.net_amount))
        .collect();
    assert_eq!(
        names,
        [(2, "航運業", -100_000.0), (1, "半導體業", -50_000.0)]
    );
    assert!(response.items[0].trade_value_ratio.is_none());
}
