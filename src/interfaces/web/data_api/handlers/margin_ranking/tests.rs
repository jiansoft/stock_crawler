//! `/market/margin-ranking` handler：融資融券增減與券資比排行的測試。

use chrono::NaiveDate;

use super::*;

fn test_period() -> FlowPeriod {
    FlowPeriod {
        data_date: NaiveDate::from_ymd_opt(2026, 10, 14).expect("合法日期"),
        from: NaiveDate::from_ymd_opt(2026, 10, 8).expect("合法日期"),
        days: 5,
    }
}

fn row(
    symbol: &str,
    market_id: i32,
    industry: &str,
    margin: [i64; 2],
    short: [i64; 2],
) -> MarginRow {
    MarginRow {
        stock_symbol: symbol.to_owned(),
        name: format!("{symbol}名稱"),
        market_id,
        industry: industry.to_owned(),
        margin_start: margin[0],
        margin_balance: margin[1],
        short_start: short[0],
        short_balance: short[1],
        closing_price: Some(20.0),
    }
}

fn universe() -> Vec<MarginRow> {
    vec![
        row("A", 2, "半導體業", [1_000, 1_500], [100, 50]),
        row("B", 4, "半導體業", [2_000, 1_200], [10, 300]),
        row("C", 2, "航運業", [0, 80], [0, 40]),
        row("D", 2, "航運業", [500, 1_000], [200, 260]),
        row("E", 4, "", [300, 300], [30, 30]),
    ]
}

fn filter<'a>(rank_by: &'a str, market_id: i32, industry: Option<&'a str>) -> RankFilter<'a> {
    RankFilter {
        rank_by,
        market: if market_id == 4 { "tpex" } else { "all" },
        market_id,
        industry,
        limit: 2,
    }
}

fn symbols(response: &MarginRankingResponse) -> Vec<(usize, &str)> {
    response
        .items
        .iter()
        .map(|item| (item.rank, item.stock_symbol.as_str()))
        .collect()
}

/// 融資增加：同增減依代號、limit 截斷但 matched 不變；增減、百分比、券資比的換算。
#[test]
fn build_response_ranks_margin_increase() {
    let response = build_response(
        test_period(),
        filter("margin_increase", 0, None),
        universe(),
    );
    assert_eq!(response.data_as_of, "2026-10-14");
    assert_eq!(response.from, "2026-10-08");
    assert_eq!(response.days, 5);
    assert_eq!(response.rank_by, "margin_increase");
    assert_eq!(response.matched, 3, "B 減少、E 不變不列入");
    assert_eq!(symbols(&response), [(1, "A"), (2, "D")]);
    let a = &response.items[0];
    assert_eq!((a.margin_balance, a.margin_change), (1_500, 500));
    assert_eq!(a.margin_change_percent, Some(50.0));
    assert_eq!((a.short_balance, a.short_change), (50, -50));
    assert_eq!(a.short_margin_ratio, Some(3.33));
    assert_eq!(a.market, "twse");
    assert_eq!(a.name, "A名稱");
    assert_eq!(a.closing_price, Some(20.0));
}

/// 其餘四種排序、期初為 0 的百分比為 null、券資比需融資至少 100 張、市場與產業篩選。
#[test]
fn build_response_other_rankings_and_filters() {
    let decrease = build_response(
        test_period(),
        filter("margin_decrease", 0, None),
        universe(),
    );
    assert_eq!(symbols(&decrease), [(1, "B")]);
    assert_eq!(decrease.items[0].margin_change, -800);
    assert_eq!(decrease.items[0].margin_change_percent, Some(-40.0));

    let short_up = build_response(test_period(), filter("short_increase", 0, None), universe());
    assert_eq!(symbols(&short_up), [(1, "B"), (2, "D")]);
    let short_down = build_response(test_period(), filter("short_decrease", 0, None), universe());
    assert_eq!(symbols(&short_down), [(1, "A")]);

    // C 券資比 50% 最高，但融資只有 80 張不列入；D 26% 高於 B 25%。
    let ratio = build_response(
        test_period(),
        filter("short_margin_ratio", 0, None),
        universe(),
    );
    assert_eq!(symbols(&ratio), [(1, "D"), (2, "B")]);
    assert_eq!(ratio.items[0].short_margin_ratio, Some(26.0));
    assert_eq!(ratio.matched, 4);

    let new_margin = build_response(
        test_period(),
        filter("margin_increase", 0, Some("航運業")),
        universe(),
    );
    assert_eq!(new_margin.industry.as_deref(), Some("航運業"));
    assert_eq!(symbols(&new_margin), [(1, "D"), (2, "C")]);
    assert_eq!(new_margin.items[1].margin_change_percent, None);

    let otc = build_response(test_period(), filter("short_increase", 4, None), universe());
    assert_eq!(otc.market, "tpex");
    assert_eq!(symbols(&otc), [(1, "B")]);
}
