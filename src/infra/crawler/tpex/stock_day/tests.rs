//! 櫃買中心個股日成交資訊（`afterTrading/tradingStock`）的測試。

use super::*;
use rust_decimal_macros::dec;

fn fixture() -> TradingStockResponse {
    let raw = include_str!("../testdata/trading_stock_00679B_201903.json");
    serde_json::from_str(raw).expect("fixture 應為合法 JSON")
}

fn row(date: &str, close: &str) -> Vec<String> {
    [
        date, "1,000", "10,000", "10.00", "10.50", "9.90", close, "+0.20", "3",
    ]
    .iter()
    .map(|value| (*value).to_owned())
    .collect()
}

fn response(stat: &str, rows: Vec<Vec<String>>) -> TradingStockResponse {
    TradingStockResponse {
        stat: Some(stat.to_owned()),
        tables: Some(vec![TradingStockTable {
            fields: None,
            data: Some(rows),
        }]),
    }
}

#[test]
fn parse_maps_every_row_and_converts_thousand_units() {
    let quotes = parse_trading_stock_response(&fixture(), "00679B");
    assert_eq!(quotes.len(), 5);

    let first = &quotes[0];
    assert_eq!(first.symbol, "00679B");
    assert_eq!(
        first.date,
        NaiveDate::from_ymd_opt(2019, 3, 4).expect("日期")
    );
    assert_eq!(first.opening_price, dec!(37.96));
    assert_eq!(first.highest_price, dec!(38.09));
    assert_eq!(first.lowest_price, dec!(37.75));
    assert_eq!(first.closing_price, dec!(38.00));
    assert_eq!(first.change, dec!(-0.69));
    // 成交仟股與成交仟元要乘一千，才和每日收盤流程寫入的股數、元同單位。
    assert_eq!(first.trading_volume, dec!(1134000));
    assert_eq!(first.trade_value, dec!(43122000));
    assert_eq!(first.transaction, dec!(370));

    let expected = (dec!(-0.69) / dec!(38.69)) * Decimal::ONE_HUNDRED;
    assert_eq!(first.change_range, expected);
}

#[test]
fn stat_is_compared_case_insensitively_and_other_values_mean_no_data() {
    assert_eq!(
        parse_trading_stock_response(&response("OK", vec![row("108/03/04", "10.20")]), "00679B")
            .len(),
        1
    );
    assert!(
        parse_trading_stock_response(
            &response("error", vec![row("108/03/04", "10.20")]),
            "00679B"
        )
        .is_empty()
    );
    let missing_stat = TradingStockResponse {
        stat: None,
        tables: None,
    };
    assert!(parse_trading_stock_response(&missing_stat, "00679B").is_empty());
}

/// 尚未掛牌的月份：stat 仍是 ok，但資料表是空的。
#[test]
fn empty_table_is_treated_as_no_data() {
    assert!(parse_trading_stock_response(&response("ok", vec![]), "00679B").is_empty());
    let no_tables = TradingStockResponse {
        stat: Some("ok".to_owned()),
        tables: Some(vec![]),
    };
    assert!(parse_trading_stock_response(&no_tables, "00679B").is_empty());
}

#[test]
fn malformed_rows_and_non_positive_closes_are_skipped() {
    let quotes = parse_trading_stock_response(
        &response(
            "ok",
            vec![
                vec!["108/03/04".to_owned(), "1".to_owned()],
                row("2019-03-05", "10.20"),
                row("108/03/06", "--"),
                row("108/03/07", "0.00"),
                row("108/03/08", "10.30"),
            ],
        ),
        "00679B",
    );

    assert_eq!(quotes.len(), 1);
    assert_eq!(quotes[0].closing_price, dec!(10.30));
}

#[test]
fn parse_roc_date_rejects_gregorian_years() {
    assert_eq!(
        parse_roc_date("108/03/04"),
        NaiveDate::from_ymd_opt(2019, 3, 4)
    );
    assert_eq!(
        parse_roc_date(" 99/01/04 "),
        NaiveDate::from_ymd_opt(2010, 1, 4)
    );
    assert_eq!(parse_roc_date("2019/03/04"), None);
    assert_eq!(parse_roc_date("108/13/04"), None);
    assert_eq!(parse_roc_date("--"), None);
}

/// 本益比、股價淨值比與最佳買賣揭示不在此來源，必須維持 0。
#[test]
fn fields_absent_from_this_source_stay_zero() {
    let quotes = parse_trading_stock_response(&fixture(), "00679B");
    let quote = &quotes[0];

    assert_eq!(quote.price_earning_ratio, Decimal::ZERO);
    assert_eq!(quote.price_to_book_ratio, Decimal::ZERO);
    assert_eq!(quote.last_best_bid_price, Decimal::ZERO);
    assert_eq!(quote.last_best_ask_price, Decimal::ZERO);
}
