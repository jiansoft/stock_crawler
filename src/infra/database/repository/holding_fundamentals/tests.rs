//! 持股基本面（最新月營收、最近一季三率與近四季自由現金流）的 PostgreSQL 查詢的測試。

use super::*;

/// 三率變化只在本季與去年同季都有值時才算；連續年減月數超出 u8 時視為查不到。
#[test]
fn row_converts_to_domain_fundamentals() {
    let fundamentals = HoldingFundamentals::from(FundamentalsRow {
        stock_symbol: "2330".to_owned(),
        revenue_month: Some(202609),
        revenue_yoy: Some(Decimal::from(-5)),
        revenue_decline_months: Some(3),
        margin_year: Some(2026),
        margin_quarter: Some(2),
        gross_margin: Some(40.0),
        operating_margin: Some(-1.5),
        net_margin: Some(2.0),
        last_year_gross_margin: Some(45.0),
        last_year_operating_margin: None,
        last_year_net_margin: Some(4.0),
        free_cash_flow_ttm: Some(-120),
    });
    assert_eq!(fundamentals.margin_period, Some((2026, 2)));
    assert_eq!(fundamentals.signals.revenue_decline_months, Some(3));
    assert_eq!(fundamentals.signals.free_cash_flow_ttm, Some(-120));
    assert_eq!(
        fundamentals.signals.margin_changes,
        [Some(-5.0), None, Some(-2.0)]
    );
    let overflow = HoldingFundamentals::from(FundamentalsRow {
        revenue_decline_months: Some(-1),
        ..Default::default()
    });
    assert_eq!(overflow.signals.revenue_decline_months, None);
}

/// 沒有代號時不查資料庫。
#[tokio::test]
async fn empty_symbols_skip_the_query() {
    let fundamentals = PgPortfolioRepository::new()
        .fetch_holding_fundamentals(&[])
        .await
        .expect("空清單");
    assert!(fundamentals.is_empty());
}
