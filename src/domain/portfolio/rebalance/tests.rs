//! 拆出的既有回歸測試，保持原有案例與斷言。

use super::*;

fn sample_positions() -> Vec<CurrentPosition> {
    vec![
        CurrentPosition {
            symbol: "2330".to_string(),
            name: "台積電".to_string(),
            shares: 10_000,
            current_price: 1000.0,
            industry: "半導體業".to_string(),
            is_etf: false,
        },
        CurrentPosition {
            symbol: "2317".to_string(),
            name: "鴻海".to_string(),
            shares: 20_000,
            current_price: 200.0,
            industry: "其他電子業".to_string(),
            is_etf: false,
        },
    ]
}

#[test]
fn rebalance_tolerance_band_prevents_small_trades() {
    // 總市值：10000*1000 + 20000*200 = 10,000,000 + 4,000,000 = 14,000,000
    // 台積電權重：10M / 14M = 71.43%
    // 若目標設為 72.0%（偏離 +0.57% < tolerance 2.0%），應建議 hold 不下單
    let targets = vec![("2330".to_string(), 72.0), ("2317".to_string(), 28.0)];
    let params = RebalanceParams {
        tolerance_percent: 2.0,
        ..Default::default()
    };

    let result = calculate_rebalance(
        TargetDimension::Stock,
        &targets,
        &sample_positions(),
        &[],
        &params,
    )
    .expect("試算成功");

    let tsmc = result
        .stock_items
        .iter()
        .find(|i| i.symbol == "2330")
        .unwrap();
    assert_eq!(tsmc.action, "hold");
    assert_eq!(tsmc.suggested_shares, 0);
}

#[test]
fn rebalance_sell_capped_by_held_shares() {
    // 台積電現有 10,000 股，若目標權重設為 0%，建議賣出股數不得超過現有 10,000 股
    let targets = vec![("2330".to_string(), 0.0), ("2317".to_string(), 100.0)];
    let params = RebalanceParams::default();

    let result = calculate_rebalance(
        TargetDimension::Stock,
        &targets,
        &sample_positions(),
        &[],
        &params,
    )
    .expect("試算成功");

    let tsmc = result
        .stock_items
        .iter()
        .find(|i| i.symbol == "2330")
        .unwrap();
    assert_eq!(tsmc.action, "sell");
    assert!(tsmc.suggested_shares <= tsmc.current_shares);
    assert_eq!(tsmc.suggested_shares, 10_000);
}

#[test]
fn rebalance_lot_size_odd_vs_round_lot() {
    // 當 lot_size = 1 時，股數不強制為 1000 之倍數；當 lot_size = 1000 時必為 1000 倍數
    let targets = vec![("2330".to_string(), 50.0), ("2317".to_string(), 50.0)];

    let params_round = RebalanceParams {
        lot_size: 1000,
        ..Default::default()
    };
    let res_round = calculate_rebalance(
        TargetDimension::Stock,
        &targets,
        &sample_positions(),
        &[],
        &params_round,
    )
    .unwrap();
    for item in &res_round.stock_items {
        assert_eq!(item.suggested_shares % 1000, 0);
    }

    let params_odd = RebalanceParams {
        lot_size: 1,
        ..Default::default()
    };
    let res_odd = calculate_rebalance(
        TargetDimension::Stock,
        &targets,
        &sample_positions(),
        &[],
        &params_odd,
    )
    .unwrap();
    // 零股模式計算正常
    assert!(res_odd.stock_items.iter().any(|i| i.suggested_shares > 0));
}

#[test]
fn rebalance_cash_shortage_reports_additional_cash() {
    // 當買入標的所需總資金超過可用資本（現有現金 ＋ 賣出淨額）時，
    // cash_summary 應正確計算出不足需新增之金額（required_additional_cash > 0）
    let targets = vec![("2330".to_string(), 100.0)];
    let positions = vec![CurrentPosition {
        symbol: "2330".to_string(),
        name: "台積電".to_string(),
        shares: 0,
        current_price: 1000.0,
        industry: "半導體業".to_string(),
        is_etf: false,
    }];
    let params = RebalanceParams {
        available_cash: 500_000.0, // 總資產 50 萬，目標 100% 買 1 張台積電需約 100 萬
        ..Default::default()
    };

    let result =
        calculate_rebalance(TargetDimension::Stock, &targets, &positions, &[], &params).unwrap();

    assert_eq!(result.stock_items[0].suggested_shares, 1000);
    assert!(result.cash_summary.required_additional_cash > 0.0);
    assert_eq!(result.cash_summary.remaining_cash, 0.0);
}

#[test]
fn rebalance_rejects_target_sum_exceeding_100() {
    let targets = vec![("2330".to_string(), 70.0), ("2317".to_string(), 35.0)];
    let params = RebalanceParams::default();
    let err = calculate_rebalance(
        TargetDimension::Stock,
        &targets,
        &sample_positions(),
        &[],
        &params,
    )
    .unwrap_err();
    assert!(err.to_string().contains("不得超過 100%"));
}

#[test]
fn rebalance_empty_portfolio_handled_gracefully() {
    let targets = vec![("2330".to_string(), 50.0)];
    let params = RebalanceParams::default();
    let result = calculate_rebalance(TargetDimension::Stock, &targets, &[], &[], &params).unwrap();
    assert_eq!(result.total_portfolio_value, 0.0);
    assert!(result.stock_items.is_empty());
}

#[test]
fn rebalance_industry_dimension_includes_candidates() {
    let targets = vec![
        ("半導體業".to_string(), 50.0),
        ("其他電子業".to_string(), 50.0),
    ];
    let params = RebalanceParams::default();
    let result = calculate_rebalance(
        TargetDimension::Industry,
        &targets,
        &sample_positions(),
        &[],
        &params,
    )
    .unwrap();

    assert_eq!(result.industry_items.len(), 2);
    let semi = result
        .industry_items
        .iter()
        .find(|i| i.industry == "半導體業")
        .unwrap();
    assert_eq!(semi.candidate_stocks, vec!["2330"]);
}

#[test]
fn rebalance_etf_look_through_aggregates_direct_and_indirect() {
    let positions = vec![
        CurrentPosition {
            symbol: "2330".to_string(),
            name: "台積電".to_string(),
            shares: 1_000,
            current_price: 1000.0, // 直接持股 1,000,000
            industry: "半導體業".to_string(),
            is_etf: false,
        },
        CurrentPosition {
            symbol: "0050".to_string(),
            name: "元大台灣50".to_string(),
            shares: 10_000,
            current_price: 200.0, // ETF 持股 2,000,000
            industry: "ETF".to_string(),
            is_etf: true,
        },
    ];

    let etf_components = vec![EtfLookThroughComponent {
        stock_symbol: "2330".to_string(),
        name: "台積電".to_string(),
        weight_percent: 55.0, // 0050 內台積電占 55% -> 間接持有 2,000,000 * 55% = 1,100,000
        etf_symbol: "0050".to_string(),
        industry: "半導體業".to_string(),
    }];

    let targets = vec![("2330".to_string(), 60.0)];
    let params = RebalanceParams::default();
    let result = calculate_rebalance(
        TargetDimension::EtfLookThrough,
        &targets,
        &positions,
        &etf_components,
        &params,
    )
    .unwrap();

    let tsmc = result
        .etf_items
        .iter()
        .find(|i| i.stock_symbol == "2330")
        .unwrap();
    assert_eq!(tsmc.direct_value, 1_000_000.0);
    assert_eq!(tsmc.indirect_value, 1_100_000.0);
    assert_eq!(tsmc.total_value, 2_100_000.0);
}
