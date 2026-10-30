//! 持股摘要的單元測試。

use rust_decimal_macros::dec;

use super::*;

fn position(symbol: &str, industry: Option<i32>, shares: i64, cost: Decimal) -> Position {
    Position {
        stock_symbol: symbol.to_owned(),
        name: format!("{symbol} 名稱"),
        industry_id: industry,
        industry_name: industry.map(|id| format!("產業{id}")).unwrap_or_default(),
        shares,
        cost,
        dividends_received: Decimal::ZERO,
        closing_price: None,
        dividend_yield_percent: None,
        f_score: None,
        pe_ratio: None,
        pb_ratio: None,
    }
}

/// 本益比與股價淨值比用市值加權的調和平均；非正值（虧損）不列入，也不算覆蓋。
#[test]
fn summarize_uses_harmonic_mean_for_valuation_ratios() {
    let positions = vec![
        Position {
            closing_price: Some(dec!(100)),
            pe_ratio: Some(dec!(10)),
            pb_ratio: Some(dec!(1)),
            ..position("1101", Some(1), 1_000, dec!(80_000))
        },
        Position {
            closing_price: Some(dec!(100)),
            pe_ratio: Some(dec!(40)),
            pb_ratio: Some(dec!(4)),
            ..position("2330", Some(24), 1_000, dec!(60_000))
        },
        Position {
            closing_price: Some(dec!(100)),
            pe_ratio: Some(Decimal::ZERO),
            ..position("2603", Some(15), 1_000, dec!(90_000))
        },
    ];
    let summary = summarize(positions);
    // 200,000 ÷ (100,000/10 + 100,000/40)＝16；覆蓋 2/3 市值。
    assert_eq!(
        summary.weighted_pe,
        Some(WeightedMetric {
            value: 16.0,
            coverage_percent: 66.67
        })
    );
    assert_eq!(summary.weighted_pb.map(|metric| metric.value), Some(1.6));
}

/// 市值、權重、損益、產業分布與市值加權指標。
#[test]
fn summarize_weights_by_market_value() {
    let positions = vec![
        Position {
            closing_price: Some(dec!(100)),
            dividend_yield_percent: Some(dec!(4)),
            f_score: Some(8),
            dividends_received: dec!(500),
            ..position("1101", Some(1), 1_000, dec!(80_000))
        },
        Position {
            closing_price: Some(dec!(50)),
            dividend_yield_percent: Some(dec!(2)),
            ..position("2330", Some(24), 1_000, dec!(60_000))
        },
        Position {
            closing_price: Some(dec!(25)),
            f_score: Some(5),
            ..position("1102", Some(1), 2_000, dec!(40_000))
        },
    ];
    let summary = summarize(positions);

    // 市值 100,000＋50,000＋50,000＝200,000；成本 180,000。
    assert_eq!((summary.market_value, summary.cost), (200_000.0, 180_000.0));
    assert_eq!(summary.unrealized_gain, 20_000.0);
    assert_eq!(summary.unrealized_gain_percent, Some(11.11));
    assert_eq!(summary.dividends_received, 500.0);
    // 殖利率只看前兩檔：(4×100k＋2×50k)/150k＝3.33，覆蓋 75%。
    assert_eq!(
        summary.weighted_dividend_yield,
        Some(WeightedMetric {
            value: 3.33,
            coverage_percent: 75.0
        })
    );
    // F 分數：(8×100k＋5×50k)/150k＝7。
    assert_eq!(
        summary.weighted_f_score.map(|metric| metric.value),
        Some(7.0)
    );

    assert_eq!(summary.industries.len(), 2);
    assert_eq!(summary.industries[0].industry_id, Some(1));
    assert_eq!(
        (
            summary.industries[0].weight_percent,
            summary.industries[0].symbols
        ),
        (75.0, 2)
    );
    assert_eq!(summary.industries[1].weight_percent, 25.0);

    let first = &summary.holdings[0];
    assert_eq!(first.position.stock_symbol, "1101");
    assert_eq!(
        (
            first.weight_percent,
            first.unrealized_gain,
            first.unrealized_gain_percent
        ),
        (Some(50.0), Some(20_000.0), Some(25.0))
    );
    // 同市值依代號排序。
    assert_eq!(summary.holdings[1].position.stock_symbol, "1102");
    assert_eq!(summary.holdings[1].unrealized_gain_percent, Some(25.0));
    assert_eq!(summary.holdings[2].unrealized_gain_percent, Some(-16.67));
}

/// 沒有收盤價的部位不計市值與損益；全為配股（成本 0）沒有報酬率；沒有任何指標時加權為 null。
#[test]
fn summarize_handles_missing_price_and_zero_cost() {
    let positions = vec![
        position("9999", None, 500, dec!(10_000)),
        Position {
            closing_price: Some(dec!(10)),
            ..position("2887", Some(17), 67, Decimal::ZERO)
        },
    ];
    let summary = summarize(positions);
    assert_eq!((summary.market_value, summary.cost), (670.0, 10_000.0));
    // 損益只算有報價的部位：670 − 0，成本 0 → 沒有報酬率。
    assert_eq!(summary.unrealized_gain, 670.0);
    assert_eq!(summary.unrealized_gain_percent, None);
    assert_eq!(summary.weighted_dividend_yield, None);
    assert_eq!(summary.weighted_f_score, None);
    assert_eq!((summary.weighted_pe, summary.weighted_pb), (None, None));

    let priced = &summary.holdings[0];
    assert_eq!(priced.position.stock_symbol, "2887");
    assert_eq!(priced.unrealized_gain_percent, None);
    let unpriced = &summary.holdings[1];
    assert_eq!(
        (
            unpriced.market_value,
            unpriced.weight_percent,
            unpriced.unrealized_gain
        ),
        (None, None, None)
    );
    let unknown_industry = summary
        .industries
        .iter()
        .find(|industry| industry.industry_id.is_none())
        .expect("未知產業也要列出");
    assert_eq!(
        (
            unknown_industry.market_value,
            unknown_industry.weight_percent
        ),
        (0.0, 0.0)
    );
}

/// 沒有持股：全部為 0、權重與報酬率為 null。
#[test]
fn summarize_empty_portfolio() {
    let summary = summarize(Vec::new());
    assert_eq!((summary.symbols, summary.market_value), (0, 0.0));
    assert_eq!(summary.unrealized_gain_percent, None);
    assert!(summary.industries.is_empty() && summary.holdings.is_empty());
}
