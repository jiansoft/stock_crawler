//! 股利實體的單元測試。

use super::*;
use rust_decimal_macros::dec;

/// 1235 興泰 2026-09-24 同日除權息：現金 0.5、股票 0.5 元，前日收盤 41.15。
#[test]
fn combines_cash_and_stock_dividend() {
    assert_eq!(
        ex_rights_reference_price(dec!(41.15), dec!(0.5), dec!(0.5)),
        Some(dec!(38.71))
    );
}

/// 2542 興富發 2026-09-23 除息 4 元：前日收盤 45.45。
#[test]
fn cash_only() {
    assert_eq!(
        ex_rights_reference_price(dec!(45.45), dec!(4), Decimal::ZERO),
        Some(dec!(41.45))
    );
}

#[test]
fn stock_only() {
    assert_eq!(
        ex_rights_reference_price(dec!(110), Decimal::ZERO, dec!(1)),
        Some(dec!(100))
    );
}

#[test]
fn none_without_dividend_or_close() {
    assert_eq!(
        ex_rights_reference_price(dec!(41.15), Decimal::ZERO, Decimal::ZERO),
        None
    );
    assert_eq!(
        ex_rights_reference_price(Decimal::ZERO, dec!(0.5), Decimal::ZERO),
        None
    );
    assert_eq!(
        ex_rights_reference_price(dec!(1), dec!(2), Decimal::ZERO),
        None
    );
}

fn info(cash_today: bool, stock_today: bool) -> StockDividendInfo {
    StockDividendInfo {
        stock_symbol: "1235".to_string(),
        name: "興泰".to_string(),
        stock_industry_id: 1,
        cash_dividend: dec!(0.5),
        stock_dividend: dec!(0.3),
        sum: dec!(0.8),
        closing_price: dec!(41.15),
        dividend_yield: Decimal::ZERO,
        cash_dividend_yield: Decimal::ZERO,
        is_cash_ex_dividend_on_date: cash_today,
        is_stock_ex_dividend_on_date: stock_today,
        quarter: String::new(),
    }
}

/// 期別決定一年配息次數；年配與無法辨識的期別不算多次配息。
#[test]
fn payout_frequency_follows_the_period_label() {
    let with = |quarter: &str| StockDividendInfo {
        quarter: quarter.to_string(),
        ..info(true, false)
    };
    assert_eq!(with("M07").payout_frequency(), Some((12, "月配")));
    assert_eq!(with("Q4").payout_frequency(), Some((4, "季配")));
    assert_eq!(with("H1").payout_frequency(), Some((2, "半年配")));
    for quarter in ["", "A", "M", "Qx", "X1"] {
        assert_eq!(with(quarter).payout_frequency(), None, "{quarter:?}");
    }
}

/// 年化殖利率用未捨入的股利與價格乘上配息次數（00953B 月配 0.067 元、9.49 元 → 8.47%）。
#[test]
fn annualized_yield_multiplies_by_payouts_per_year() {
    let monthly = StockDividendInfo {
        cash_dividend: dec!(0.067),
        stock_dividend: Decimal::ZERO,
        sum: dec!(0.067),
        closing_price: dec!(9.49),
        quarter: "M09".to_string(),
        ..info(true, false)
    };
    assert_eq!(monthly.annualized_yield(), Some(dec!(8.47)));

    let quarterly = StockDividendInfo {
        sum: dec!(0.5),
        closing_price: dec!(20),
        quarter: "Q3".to_string(),
        ..info(true, false)
    };
    assert_eq!(quarterly.annualized_yield(), Some(dec!(10)));

    assert_eq!(info(true, false).annualized_yield(), None, "年配");
    let no_price = StockDividendInfo {
        closing_price: Decimal::ZERO,
        ..monthly
    };
    assert_eq!(no_price.annualized_yield(), None);
}

/// 除息、除權不同天時，只計入當天生效的那一項。
#[test]
fn effective_on_date_keeps_only_today_events() {
    assert_eq!(info(true, true).effective_on_date(), (dec!(0.5), dec!(0.3)));
    assert_eq!(
        info(true, false).effective_on_date(),
        (dec!(0.5), Decimal::ZERO)
    );
    assert_eq!(
        info(false, true).effective_on_date(),
        (Decimal::ZERO, dec!(0.3))
    );
}

fn unannounced(cash: Decimal, stock: Decimal) -> Dividend {
    Dividend {
        serial: 0,
        year: 2026,
        year_of_dividend: 2025,
        quarter: String::new(),
        security_code: "2330".to_string(),
        earnings_cash_dividend: cash,
        capital_reserve_cash_dividend: Decimal::ZERO,
        cash_dividend: cash,
        earnings_stock_dividend: stock,
        capital_reserve_stock_dividend: Decimal::ZERO,
        stock_dividend: stock,
        sum: cash + stock,
        payout_ratio_cash: Decimal::ZERO,
        payout_ratio_stock: Decimal::ZERO,
        payout_ratio: Decimal::ZERO,
        ex_dividend_date_cash: "尚未公布".to_string(),
        ex_dividend_date_stock: "尚未公布".to_string(),
        payable_date_cash: "尚未公布".to_string(),
        payable_date_stock: "尚未公布".to_string(),
        created_time: Local::now(),
        updated_time: Local::now(),
    }
}

fn dates(dividend: &Dividend) -> [&str; 4] {
    [
        &dividend.ex_dividend_date_cash,
        &dividend.payable_date_cash,
        &dividend.ex_dividend_date_stock,
        &dividend.payable_date_stock,
    ]
}

/// 只配現金：除權日與股票股利發放日不會有日期；只配股票則相反。
#[test]
fn clear_dates_of_zero_component_marks_the_missing_side() {
    let mut cash_only = unannounced(dec!(3), Decimal::ZERO);
    cash_only.clear_dates_of_zero_component();
    assert_eq!(dates(&cash_only), ["尚未公布", "尚未公布", "-", "-"]);

    let mut stock_only = unannounced(Decimal::ZERO, dec!(1));
    stock_only.clear_dates_of_zero_component();
    assert_eq!(dates(&stock_only), ["-", "-", "尚未公布", "尚未公布"]);
}

/// 兩項都配、或金額還沒公告（都是 0）、或已有實際日期時，都不動。
#[test]
fn clear_dates_of_zero_component_keeps_everything_else() {
    let mut both = unannounced(dec!(0.5), dec!(2));
    both.clear_dates_of_zero_component();
    assert_eq!(dates(&both), ["尚未公布"; 4]);

    let mut pending = unannounced(Decimal::ZERO, Decimal::ZERO);
    pending.clear_dates_of_zero_component();
    assert_eq!(dates(&pending), ["尚未公布"; 4]);

    let mut dated = unannounced(dec!(3), Decimal::ZERO);
    dated.ex_dividend_date_stock = "2026-07-01".to_string();
    dated.clear_dates_of_zero_component();
    assert_eq!(dates(&dated)[2], "2026-07-01");
}
