//! `/market/relative-strength` handler：相對強度排行（RS）的測試。

use super::*;
use crate::domain::quote::relative_strength::score_universe;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("合法日期")
}

fn row(symbol: &str, market_id: i32, industry: &str, returns: [f64; 3]) -> UniverseRow {
    UniverseRow {
        stock_symbol: symbol.to_owned(),
        name: format!("{symbol}名稱"),
        market_id,
        industry: industry.to_owned(),
        return_20d: None,
        return_60d: returns[0],
        return_120d: returns[1],
        return_240d: returns[2],
        closing_price: Some(10.0),
    }
}

fn params(
    market: Option<&str>,
    industry: Option<&str>,
    order: Option<&str>,
) -> RelativeStrengthParams {
    RelativeStrengthParams {
        date: None,
        market: market.map(str::to_owned),
        industry: industry.map(str::to_owned),
        symbol: None,
        order: order.map(str::to_owned),
        limit: None,
    }
}

/// 以全體計分排名，篩選只影響輸出；同分同名次；asc 由弱到強；limit 截斷但 matched 不變。
#[test]
fn build_response_ranks_whole_universe_then_filters() {
    let rows = vec![
        row("A", 2, "半導體業", [10.0, 20.0, 30.0]),
        row("B", 4, "半導體業", [-10.0, -10.0, -10.0]),
        row("C", 2, "航運業", [0.0, 0.0, 0.0]),
        row("D", 4, "航運業", [0.0, 0.0, 0.0]),
        row("E", 2, "半導體業", [50.0, 50.0, 50.0]),
    ];
    let all = build_response(
        day(2026, 10, 13),
        &params(None, None, None),
        0,
        score_universe(rows.clone()),
    );
    assert_eq!(all.data_as_of, "2026-10-13");
    assert_eq!(all.market, "all");
    assert_eq!(all.order, "desc");
    assert_eq!(all.universe, 5);
    assert_eq!(all.matched, 5);
    let ranked: Vec<(&str, usize, u8)> = all
        .items
        .iter()
        .map(|item| (item.stock_symbol.as_str(), item.rank, item.rs_score))
        .collect();
    assert_eq!(
        ranked,
        [
            ("E", 1, 99),
            ("A", 2, 74),
            ("C", 3, 25),
            ("D", 3, 25),
            ("B", 5, 1)
        ]
    );
    // A：0.4×10 + 0.3×20 + 0.3×30 = 19。
    assert_eq!(all.items[1].weighted_return, 19.0);
    assert_eq!(all.items[1].market, "twse");
    assert_eq!(all.items[1].industry, "半導體業");

    let otc = build_response(
        day(2026, 10, 13),
        &params(Some("tpex"), None, Some("asc")),
        4,
        score_universe(rows.clone()),
    );
    assert_eq!(otc.order, "asc");
    let symbols: Vec<&str> = otc.items.iter().map(|i| i.stock_symbol.as_str()).collect();
    assert_eq!(symbols, ["B", "D"]);
    assert_eq!(otc.items[1].rank, 3, "名次仍以全體計算");

    let mut semi = params(None, Some("半導體業"), None);
    semi.limit = Some(1);
    let semi = build_response(day(2026, 10, 13), &semi, 0, score_universe(rows.clone()));
    assert_eq!(semi.matched, 3);
    assert_eq!(semi.items.len(), 1);
    assert_eq!(semi.items[0].stock_symbol, "E");

    let mut one = params(None, None, None);
    one.symbol = Some("C".to_owned());
    let one = build_response(day(2026, 10, 13), &one, 0, score_universe(rows));
    assert_eq!(one.items.len(), 1);
    assert_eq!((one.items[0].rank, one.items[0].rs_score), (3, 25));
}
