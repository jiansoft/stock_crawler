//! 報價實體的測試。

use super::*;
use rust_decimal_macros::dec;

#[test]
fn test_calculate_change_range() {
    // 建立測試用日報價，利用結構體初始化設定收盤價
    let mut quote = DailyQuote {
        closing_price: dec!(110),
        ..Default::default()
    };

    // 昨收為 100
    quote.calculate_change_range(dec!(100));
    // 預期上漲 10 元
    assert_eq!(quote.change, dec!(10));
    // 預期漲幅 10%
    assert_eq!(quote.change_range, dec!(10));

    // 昨收為 0（非法）
    quote.calculate_change_range(dec!(0));
    // 預期清零
    assert_eq!(quote.change, dec!(0));
    assert_eq!(quote.change_range, dec!(0));
}

#[test]
fn test_is_limit_move() {
    // 正常波動 5.5%
    let quote_normal = DailyQuote {
        change_range: dec!(5.5),
        ..Default::default()
    };
    assert!(!quote_normal.is_limit_move());

    // 漲停 9.95%
    let quote_up = DailyQuote {
        change_range: dec!(9.95),
        ..Default::default()
    };
    assert!(quote_up.is_limit_move());

    // 跌停 -10.0%
    let quote_down = DailyQuote {
        change_range: dec!(-10.0),
        ..Default::default()
    };
    assert!(quote_down.is_limit_move());
}
