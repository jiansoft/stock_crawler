//! 報價防腐層轉換的測試。

use super::*;
use rust_decimal_macros::dec;

#[test]
fn test_quote_acl_mapping() {
    let dto = DailyQuoteDto {
        symbol: "2330".to_string(),
        date: NaiveDate::from_ymd_opt(2026, 6, 5).unwrap(),
        opening_price: dec!(900.0),
        highest_price: dec!(910.0),
        lowest_price: dec!(895.0),
        closing_price: dec!(905.0),
        change: dec!(5.0),
        change_range: dec!(0.55),
        trading_volume: dec!(10000),
        trade_value: dec!(9000000),
        transaction: dec!(500),
        price_earning_ratio: dec!(20.5),
        price_to_book_ratio: dec!(5.2),
        last_best_bid_price: dec!(904.0),
        last_best_bid_volume: dec!(10),
        last_best_ask_price: dec!(905.0),
        last_best_ask_volume: dec!(20),
    };

    let cmd = QuoteAclMapper::from_dto(&dto);
    assert_eq!(cmd.symbol, "2330");
    assert_eq!(cmd.closing_price, dec!(905.0));

    let entity = QuoteAclMapper::from_command(&cmd);
    assert_eq!(entity.stock_symbol, "2330");
    assert_eq!(entity.closing_price, dec!(905.0));
    assert_eq!(entity.date, NaiveDate::from_ymd_opt(2026, 6, 5).unwrap());
}
