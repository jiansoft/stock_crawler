//! 年化報酬率資料表的測試。

use super::*;
use rust_decimal_macros::dec;

fn sample_outcome() -> SimulationOutcome {
    SimulationOutcome {
        end_shares: dec!(123.45678),
        cash_received: dec!(321.5),
        end_value: dec!(12345.6789),
        total_return_pct: dec!(23.4567),
        cagr_pct: dec!(7.8901),
    }
}

fn sample_domain() -> DomainStockCagr {
    DomainStockCagr {
        date: NaiveDate::from_ymd_opt(2026, 6, 30).unwrap(),
        stock_symbol: "2330".to_string(),
        period: CagrPeriod::Y3,
        base_date: NaiveDate::from_ymd_opt(2023, 6, 30),
        base_price: Some(dec!(560.0)),
        end_price: Some(dec!(1000.0)),
        years: Some(dec!(3.001)),
        price: Some(sample_outcome()),
        total: Some(sample_outcome()),
        reinvested: None,
        first_quote_date: NaiveDate::from_ymd_opt(2000, 1, 4),
        shortfall_days: Some(0),
        data_complete: true,
        has_anomaly: false,
        dividend_events: 12,
    }
}

#[test]
fn test_round_trip_keeps_values() {
    let domain = sample_domain();
    let row = StockCagr::from(&domain);
    let restored = row.to_domain().expect("期間代碼應可還原");

    // 口徑 A 的現金不落庫，還原時補零，故先個別比對再比其餘欄位。
    assert_eq!(restored.price.map(|o| o.cash_received), Some(Decimal::ZERO));
    assert_eq!(restored.total, domain.total);
    assert_eq!(restored.reinvested, None);
    assert_eq!(restored.date, domain.date);
    assert_eq!(restored.period, domain.period);
    assert_eq!(restored.years, domain.years);
    assert_eq!(restored.dividend_events, domain.dividend_events);
}

#[test]
fn test_incomplete_row_maps_to_none() {
    let mut domain = sample_domain();
    domain.data_complete = false;
    domain.price = None;
    domain.total = None;
    domain.reinvested = None;
    domain.base_price = None;
    domain.years = None;

    let row = StockCagr::from(&domain);
    assert!(row.total_end_value.is_none());
    assert!(row.price_cagr_pct.is_none());

    let restored = row.to_domain().expect("期間代碼應可還原");
    assert!(restored.price.is_none());
    assert!(restored.total.is_none());
    assert!(restored.reinvested.is_none());
    assert!(!restored.data_complete);
}

#[test]
fn test_unknown_period_code_is_error() {
    let mut row = StockCagr::from(&sample_domain());
    row.period = "Z9".to_string();
    assert!(row.to_domain().is_err());
}
