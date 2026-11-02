use rust_decimal_macros::dec;

use super::*;

fn default_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()
}

#[test]
fn new_sets_stock_symbol_and_default_key_fields() {
    let quote = DailyQuote::new("2330");

    assert_eq!(quote.stock_symbol, "2330");
    assert_eq!(quote.serial, 0);
    assert_eq!(quote.year, 0);
    assert_eq!(quote.month, 0);
    assert_eq!(quote.day, 0);
    assert_eq!(quote.closing_price, Decimal::ZERO);
}

#[test]
fn key_and_key_with_prefix_use_date_and_symbol() {
    let mut quote = DailyQuote::new("2330");
    quote.date = NaiveDate::from_ymd_opt(2025, 2, 3).unwrap();

    assert_eq!(quote.key(), "20250203-2330");
    assert_eq!(quote.key_with_prefix(), "DailyQuote:20250203-2330");
}

#[test]
fn from_with_exchange_twse_maps_signed_change_and_decimals() {
    let item = vec![
        "2330".to_string(),
        "台積電".to_string(),
        "1,234,000".to_string(),
        "5,678".to_string(),
        "987,654,321".to_string(),
        "950.5".to_string(),
        "960.5".to_string(),
        "945.5".to_string(),
        "955.5".to_string(),
        "<p style=color:green>-</p>".to_string(),
        "12.5".to_string(),
        "955.0".to_string(),
        "100".to_string(),
        "956.0".to_string(),
        "200".to_string(),
        "20.5".to_string(),
    ];

    let quote = DailyQuote::from_with_exchange(StockExchange::TWSE, &item);

    assert_eq!(quote.stock_symbol, "2330");
    assert_eq!(quote.trading_volume, dec!(1234000));
    assert_eq!(quote.transaction, dec!(5678));
    assert_eq!(quote.trade_value, dec!(987654321));
    assert_eq!(quote.opening_price, dec!(950.5));
    assert_eq!(quote.highest_price, dec!(960.5));
    assert_eq!(quote.lowest_price, dec!(945.5));
    assert_eq!(quote.closing_price, dec!(955.5));
    assert_eq!(quote.change, dec!(-12.5));
    assert_eq!(quote.price_earning_ratio, dec!(20.5));
    assert_eq!(quote.maximum_price_in_year_date_on, default_date());
    assert_eq!(quote.minimum_price_in_year_date_on, default_date());
}

#[test]
fn from_with_exchange_tpex_maps_quote_fields() {
    let item = vec![
        "6488".to_string(),
        "環球晶".to_string(),
        "520.5".to_string(),
        "+3.5".to_string(),
        "518.0".to_string(),
        "525.0".to_string(),
        "515.0".to_string(),
        "78,000".to_string(),
        "40,599,000".to_string(),
        "63".to_string(),
        "520.0".to_string(),
        "2".to_string(),
        "521.0".to_string(),
        "3".to_string(),
    ];

    let quote = DailyQuote::from_with_exchange(StockExchange::TPEx, &item);

    assert_eq!(quote.stock_symbol, "6488");
    assert_eq!(quote.closing_price, dec!(520.5));
    assert_eq!(quote.change, dec!(3.5));
    assert_eq!(quote.opening_price, dec!(518.0));
    assert_eq!(quote.highest_price, dec!(525.0));
    assert_eq!(quote.lowest_price, dec!(515.0));
    assert_eq!(quote.trading_volume, dec!(78000));
    assert_eq!(quote.trade_value, dec!(40599000));
    assert_eq!(quote.transaction, dec!(63));
    assert_eq!(quote.last_best_bid_price, dec!(520.0));
    assert_eq!(quote.last_best_bid_volume, dec!(2));
    assert_eq!(quote.last_best_ask_price, dec!(521.0));
    assert_eq!(quote.last_best_ask_volume, dec!(3));
}

#[test]
fn to_csv_contains_expected_columns_and_trailing_newline() {
    let mut quote = DailyQuote::new("2330");
    quote.date = NaiveDate::from_ymd_opt(2025, 2, 3).unwrap();
    quote.maximum_price_in_year_date_on = NaiveDate::from_ymd_opt(2025, 2, 1).unwrap();
    quote.minimum_price_in_year_date_on = NaiveDate::from_ymd_opt(2025, 1, 2).unwrap();
    quote.closing_price = dec!(955.5);
    quote.change = dec!(-12.5);
    quote.year = 2025;
    quote.month = 2;
    quote.day = 3;

    let csv = quote.to_csv();

    assert!(csv.ends_with('\n'));
    assert!(csv.contains("2025-02-01,2025-01-02,2025-02-03"));
    assert!(csv.contains("955.5"));
    assert!(csv.contains("-12.5"));
    assert!(csv.contains(",2330,2025,2,3"));
    assert_eq!(csv.trim_end().split(',').count(), 33);
}
