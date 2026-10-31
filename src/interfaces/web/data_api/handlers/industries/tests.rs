use rust_decimal_macros::dec;

use super::*;

fn row(
    industry_id: i32,
    symbol: &str,
    close: Decimal,
    change: Decimal,
    base_close: Option<Decimal>,
) -> IndustryRow {
    IndustryRow {
        industry_id,
        industry_name: format!("產業{industry_id}"),
        stock_symbol: symbol.to_string(),
        name: format!("股{symbol}"),
        closing_price: close,
        change_range: change,
        trade_value: dec!(100),
        price_earning_ratio: dec!(10),
        price_to_book_ratio: dec!(0),
        issued_share: 1000,
        base_close,
    }
}

/// 單日用交易所漲跌幅；區間用期初收盤價，期初沒有報價或為 0 時不列入。
#[test]
fn change_percent_uses_exchange_range_for_one_day_and_base_close_otherwise() {
    let r = row(1, "1101", dec!(110), dec!(-1.5), Some(dec!(100)));
    assert_eq!(change_percent(&r, 1), Some(dec!(-1.5)));
    assert_eq!(change_percent(&r, 5), Some(dec!(10)));
    assert_eq!(
        change_percent(&row(1, "1102", dec!(10), dec!(1), None), 5),
        None
    );
    assert_eq!(
        change_percent(&row(1, "1103", dec!(10), dec!(1), Some(dec!(0))), 5),
        None
    );
}

/// 市值加權：大型股主導方向；沒有發行股數的個股不列入，全部沒有時為 None。
#[test]
fn cap_weighted_change_weights_by_base_market_cap() {
    let big = IndustryRow {
        issued_share: 9000,
        ..row(1, "2330", dec!(110), dec!(10), None)
    };
    let small = row(1, "2303", dec!(90), dec!(-10), None);
    let no_shares = IndustryRow {
        issued_share: 0,
        ..row(1, "2302", dec!(50), dec!(50), None)
    };
    // 期初市值 900000 + 100000，期末 990000 + 90000 → +8%
    let changes = [
        (&big, dec!(10)),
        (&small, dec!(-10)),
        (&no_shares, dec!(50)),
    ];
    assert_eq!(
        cap_weighted_change(&changes).map(|v| v.round_dp(4)),
        Some(dec!(8))
    );
    assert_eq!(cap_weighted_change(&[(&no_shares, dec!(50))]), None);
    assert_eq!(cap_weighted_change(&[]), None);
}

/// 彙總：漲跌家數、中位數、成交值占比、估值只算正值、最大漲跌個股，並依加權漲跌幅排序。
#[test]
fn summarize_groups_by_industry_and_sorts_by_cap_weighted_change() {
    let mut flat = row(2, "1216", dec!(50), dec!(0), None);
    flat.trade_value = dec!(200);
    let rows = vec![
        row(1, "2330", dec!(110), dec!(3), None),
        row(1, "2303", dec!(50), dec!(-2), None),
        row(1, "2454", dec!(100), dec!(3), None),
        flat,
        IndustryRow {
            issued_share: 0,
            ..row(3, "9999", dec!(10), dec!(-1), None)
        },
    ];

    let industries = summarize(&rows, 1);
    let ids: Vec<i32> = industries.iter().map(|item| item.industry_id).collect();
    assert_eq!(ids, vec![1, 2, 3], "加權漲跌幅 None 的產業排最後");

    let semis = &industries[0];
    assert_eq!(semis.industry_name, "產業1");
    assert_eq!(
        (
            semis.stocks,
            semis.advancers,
            semis.decliners,
            semis.unchanged
        ),
        (3, 2, 1, 0)
    );
    assert_eq!(semis.median_change_percent, Some(3.0));
    assert_eq!(semis.trade_value, 300.0);
    assert_eq!(semis.trade_value_share_percent, Some(50.0));
    assert_eq!(semis.median_price_earning_ratio, Some(10.0));
    assert_eq!(semis.median_price_to_book_ratio, None, "淨值比 0 不列入");
    let gainer = semis.top_gainer.as_ref().unwrap();
    assert_eq!(
        (gainer.stock_symbol.as_str(), gainer.change_percent),
        ("2330", 3.0),
        "同漲幅取代號較小者"
    );
    assert_eq!(semis.top_loser.as_ref().unwrap().stock_symbol, "2303");

    let food = &industries[1];
    assert_eq!(
        (food.unchanged, food.cap_weighted_change_percent),
        (1, Some(0.0))
    );
    assert!(food.top_gainer.is_none() && food.top_loser.is_none());
    assert_eq!(industries[2].cap_weighted_change_percent, None);
}

/// 區間模式下期初沒有報價的個股只算檔數與成交值，不列入漲跌統計。
#[test]
fn summarize_skips_stocks_without_base_close_in_period_mode() {
    let rows = vec![
        row(1, "2330", dec!(110), dec!(0), Some(dec!(100))),
        row(1, "6999", dec!(50), dec!(9), None),
    ];
    let semis = &summarize(&rows, 20)[0];
    assert_eq!((semis.stocks, semis.advancers, semis.decliners), (2, 1, 0));
    assert_eq!(semis.median_change_percent, Some(10.0));
    assert_eq!(semis.cap_weighted_change_percent, Some(10.0));
}

/// 跌幅相同時也取代號較小者。
#[test]
fn top_loser_ties_pick_smaller_symbol() {
    let rows = vec![
        row(1, "2303", dec!(10), dec!(-2), None),
        row(1, "2301", dec!(10), dec!(-2), None),
    ];
    let loser = summarize(&rows, 1)[0].top_loser.clone().unwrap();
    assert_eq!(
        (loser.stock_symbol.as_str(), loser.change_percent),
        ("2301", -2.0)
    );
}

/// 0.005 進位成 0.01，不是銀行家捨入的 0。
#[test]
fn round2_rounds_half_away_from_zero() {
    assert_eq!(round2(dec!(0.005)), 0.01);
    assert_eq!(round2(dec!(-2.345)), -2.35);
}

/// 全市場沒有成交金額時不計算占比。
#[test]
fn summarize_without_trade_value_has_no_share() {
    let mut quiet = row(1, "2330", dec!(10), dec!(1), None);
    quiet.trade_value = Decimal::ZERO;
    assert_eq!(summarize(&[quiet], 1)[0].trade_value_share_percent, None);
}
