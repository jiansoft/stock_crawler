//! `/portfolio/etf-exposure` handler：持股穿透曝險的測試。

use super::*;
use crate::domain::portfolio::exposure::{EtfConstituent, ExposurePosition, look_through};

#[test]
fn response_rounds_money_and_limits_items() {
    let report = look_through(
        &[
            ExposurePosition {
                stock_symbol: "0050".to_owned(),
                name: "元大台灣50".to_owned(),
                market_value: Some(1_000.4),
                is_etf: true,
            },
            ExposurePosition {
                stock_symbol: "2317".to_owned(),
                name: "鴻海".to_owned(),
                market_value: Some(100.0),
                is_etf: false,
            },
        ],
        &[EtfConstituent {
            etf_symbol: "0050".to_owned(),
            as_of: chrono::NaiveDate::from_ymd_opt(2026, 9, 30).expect("日期合法"),
            holding_symbol: "2330".to_owned(),
            holding_name: "台積電".to_owned(),
            weight: 56.456,
        }],
    );
    let response = response(report, 1);
    assert_eq!(response.total_value, 1_100.0);
    assert_eq!(response.total_items, 2);
    assert_eq!(response.items.len(), 1);
    let tsmc = &response.items[0];
    assert_eq!((tsmc.symbol.as_str(), tsmc.total_value), ("2330", 565.0));
    assert_eq!(tsmc.weight, 51.33);
    assert_eq!(tsmc.sources[0].etf_symbol, "0050");
    assert_eq!(response.etfs[0].covered_weight, 56.46);
    assert_eq!(
        response.etfs[0].holdings_as_of.as_deref(),
        Some("2026-09-30")
    );
}

#[test]
fn an_empty_portfolio_has_zero_weights() {
    let response = response(ExposureReport::default(), 20);
    assert_eq!(response.total_value, 0.0);
    assert!(response.items.is_empty() && response.etfs.is_empty());
}
