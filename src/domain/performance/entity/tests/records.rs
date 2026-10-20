//! 股利事件排序、CAGR 覆蓋率與快取鍵的測試。

use super::*;

#[test]
fn dividend_event_sort_key_takes_the_earlier_of_two_dates() {
    let make = |cash: Option<NaiveDate>, stock: Option<NaiveDate>| DividendEvent {
        stock_symbol: "2330".to_string(),
        ex_dividend_date_cash: cash,
        ex_dividend_date_stock: stock,
        cash_dividend: Decimal::ZERO,
        stock_dividend: Decimal::ZERO,
    };
    let cash_day = date(2024, 6, 20);
    let stock_day = date(2024, 7, 15);

    assert_eq!(
        make(Some(cash_day), Some(stock_day)).sort_key(),
        Some(cash_day)
    );
    assert_eq!(
        make(Some(stock_day), Some(cash_day)).sort_key(),
        Some(cash_day)
    );
    assert_eq!(make(Some(cash_day), None).sort_key(), Some(cash_day));
    assert_eq!(make(None, Some(stock_day)).sort_key(), Some(stock_day));
    // 兩個日期都缺的事件不應納入模擬，因此沒有排序鍵。
    assert_eq!(make(None, None).sort_key(), None);
}

#[test]
fn coverage_ratio_uses_universe_and_positive_ratio_uses_counted() {
    let coverage = CagrCoverage {
        universe: 200,
        counted: 100,
        incomplete: 100,
        anomaly_flagged: 5,
        positive: 40,
    };
    assert_eq!(coverage.coverage_ratio(), Decimal::new(5, 1)); // 100/200
    // 分母若誤用 universe 會得到 0.2，這裡確保是 counted。
    assert_eq!(coverage.positive_ratio(), Decimal::new(4, 1)); // 40/100
}

#[test]
fn coverage_ratios_are_zero_when_denominator_is_zero() {
    let empty = CagrCoverage::default();
    assert_eq!(empty.coverage_ratio(), Decimal::ZERO);
    assert_eq!(empty.positive_ratio(), Decimal::ZERO);

    // 母體不為零但可算數為零時，正報酬比例仍必須是零而非除以零。
    let none_counted = CagrCoverage {
        universe: 10,
        ..Default::default()
    };
    assert_eq!(none_counted.coverage_ratio(), Decimal::ZERO);
    assert_eq!(none_counted.positive_ratio(), Decimal::ZERO);
}

#[test]
fn stock_cagr_key_includes_symbol_date_and_period() {
    let record = StockCagr {
        date: date(2026, 8, 7),
        stock_symbol: "2330".to_string(),
        period: CagrPeriod::Y1H,
        base_date: None,
        base_price: None,
        end_price: None,
        years: None,
        price: None,
        total: None,
        reinvested: None,
        first_quote_date: None,
        shortfall_days: None,
        data_complete: false,
        has_anomaly: false,
        dividend_events: 0,
    };
    assert_eq!(record.key(), "2330-2026-08-07-Y1H");
    assert_eq!(record.key_with_prefix(), "StockCagr:2330-2026-08-07-Y1H");
}
