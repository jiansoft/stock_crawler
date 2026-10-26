//! 拆出的既有回歸測試，保持原有案例與斷言。

use super::*;
/// 產生一筆有效且可交易的測試部位。
fn position(symbol: &str, is_etf: bool) -> CurrentPosition {
    CurrentPosition {
        symbol: symbol.into(),
        name: symbol.into(),
        shares: 10_000,
        current_price: 100.0,
        industry: if is_etf { "ETF" } else { "半導體業" }.into(),
        is_etf,
    }
}
/// ETF 市值必須依成分股產業穿透，不能將 ETF 本身當成產業。
#[test]
fn industry_looks_through_etf_and_uses_master_classification() {
    let components = [EtfLookThroughComponent {
        stock_symbol: "2330".into(),
        name: "台積電".into(),
        weight_percent: 60.0,
        etf_symbol: "FUND".into(),
        industry: "半導體業".into(),
    }];
    let result = calculate_rebalance(
        TargetDimension::Industry,
        &[("半導體業".into(), 60.0)],
        &[position("FUND", true)],
        &components,
        &RebalanceParams::default(),
    )
    .unwrap();
    assert_eq!(result.industry_items[0].current_value, 600_000.0);
    assert_eq!(result.industry_items[0].current_weight, 60.0);
    let exposure = calculate_rebalance(
        TargetDimension::EtfLookThrough,
        &[],
        &[position("FUND", true)],
        &components,
        &RebalanceParams::default(),
    )
    .unwrap();
    assert_eq!(exposure.etf_items[0].indirect_value, 600_000.0);
}
/// ETF 賣出沿用回測費率，零目標也可提出全數賣出的建議。
#[test]
fn cash_target_sells_etf_with_etf_tax() {
    let result = calculate_rebalance(
        TargetDimension::Stock,
        &[("FUND".into(), 0.0)],
        &[position("FUND", true)],
        &[],
        &RebalanceParams::default(),
    )
    .unwrap();
    assert_eq!(result.target_cash_percent, 100.0);
    assert_eq!(result.stock_items[0].suggested_shares, 10_000);
    assert_eq!(result.stock_items[0].estimated_cost, 2425.0);
}
/// 非有限值、重複 key 與超出儲存精度皆不可進入計算。
#[test]
fn rejects_invalid_weights_and_unpriced_positions() {
    for targets in [
        vec![("A".into(), f64::NAN)],
        vec![("A".into(), 33.335)],
        vec![("A".into(), 10.0), ("A".into(), 10.0)],
        vec![(" ".into(), 0.0)],
    ] {
        assert!(
            calculate_rebalance(
                TargetDimension::Stock,
                &targets,
                &[],
                &[],
                &RebalanceParams::default()
            )
            .is_err()
        );
    }
    let mut p = position("A", false);
    p.current_price = 0.0;
    assert!(
        calculate_rebalance(
            TargetDimension::Stock,
            &[],
            &[p],
            &[],
            &RebalanceParams::default()
        )
        .is_err()
    );
    for params in [
        RebalanceParams {
            available_cash: f64::INFINITY,
            ..Default::default()
        },
        RebalanceParams {
            tolerance_percent: f64::NAN,
            ..Default::default()
        },
        RebalanceParams {
            lot_size: 10,
            ..Default::default()
        },
    ] {
        assert!(calculate_rebalance(TargetDimension::Stock, &[], &[], &[], &params).is_err());
    }
}
