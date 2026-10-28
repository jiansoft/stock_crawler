use rust_decimal_macros::dec;

use super::*;

mod financial;
mod foreign;
mod notified;
mod revenue;

fn revenue_alert(symbol: &str, name: &str, yoy: Decimal, mom: Decimal) -> HoldingRevenueAlert {
    HoldingRevenueAlert {
        stock_symbol: symbol.to_string(),
        stock_name: name.to_string(),
        industry_name: Some("半導體業".to_string()),
        monthly: dec!(250000000),
        monthly_accumulated: dec!(1000000),
        compared_with_last_month: mom,
        compared_with_last_year_same_month: yoy,
        accumulated_compared_with_last_year: dec!(12.5),
        issued_share: 100_000_000,
        net_income_margin: Some(dec!(40)),
        // 錨點 EPS 3 元、錨點營收 600,000 千元 ⇒ 3 × (1,000,000 ÷ 600,000) ＝ 5 元。
        anchor_eps: Some(dec!(3)),
        anchor_accumulated_revenue: Some(dec!(600000)),
        anchor_quarter_no: Some(2),
        ttm_eps: None,
        ttm_revenue: None,
        reg_slope: None,
        reg_intercept: None,
        reg_r2: None,
        reg_quarters: None,
        previous_high: None,
        yoy_decline_months: 0,
        date: 202608,
    }
}

fn foreign_alert(
    symbol: &str,
    name: &str,
    change_20d: Option<Decimal>,
    streak: i32,
) -> HoldingForeignHoldingAlert {
    HoldingForeignHoldingAlert {
        stock_symbol: symbol.to_string(),
        stock_name: name.to_string(),
        date: NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
        share_holding_percentage: dec!(26.3700),
        change_5d: Some(dec!(1.0300)),
        change_20d,
        streak,
    }
}

fn financial_alert(last_year_eps: Option<Decimal>) -> HoldingFinancialAlert {
    HoldingFinancialAlert {
        stock_symbol: "2330".to_string(),
        stock_name: "台積電".to_string(),
        quarter: "Q2".to_string(),
        earnings_per_share: dec!(9.56),
        return_on_equity: dec!(8.4),
        gross_profit: dec!(53.1),
        operating_margin: dec!(42.5),
        net_margin: dec!(38.2),
        last_year_earnings_per_share: last_year_eps,
        last_year_gross_profit: last_year_eps.map(|_| dec!(50.8)),
        last_year_operating_margin: last_year_eps.map(|_| dec!(40)),
        last_year_net_margin: last_year_eps.map(|_| dec!(36.7)),
        year: 2026,
    }
}

#[test]
fn format_revenue_month_pads_single_digit_month() {
    assert_eq!(EventDispatcher::format_revenue_month(202608), "2026-08");
    assert_eq!(EventDispatcher::format_revenue_month(202612), "2026-12");
}

#[test]
fn with_sign_only_prefixes_positive_values() {
    assert_eq!(EventDispatcher::with_sign(dec!(33.25)), "+33.25");
    assert_eq!(EventDispatcher::with_sign(dec!(-4.1)), "-4.1");
    assert_eq!(EventDispatcher::with_sign(Decimal::ZERO), "0");
}

#[test]
fn stock_link_escapes_dots_in_url() {
    let link = EventDispatcher::stock_link("2330");

    assert_eq!(link, "[2330](https://tw\\.stock\\.yahoo\\.com/quote/2330)");
}
