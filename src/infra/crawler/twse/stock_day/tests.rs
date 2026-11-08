//! TWSE 個股日成交資訊（`STOCK_DAY`）的測試。

use super::*;
use rust_decimal_macros::dec;

fn fixture() -> StockDayResponse {
    let raw = include_str!("../testdata/stock_day_0050_202108.json");
    serde_json::from_str(raw).expect("fixture 應為合法 JSON")
}

#[test]
fn parse_stock_day_response_maps_every_traded_day() {
    let quotes = parse_stock_day_response(&fixture(), "0050");

    // fixture 共六列，其中一列是無成交的 `--`，必須被略過。
    assert_eq!(quotes.len(), 5);

    let first = &quotes[0];
    assert_eq!(first.symbol, "0050");
    assert_eq!(
        first.date,
        NaiveDate::from_ymd_opt(2021, 8, 2).expect("測試日期應合法")
    );
    assert_eq!(first.opening_price, dec!(137.00));
    assert_eq!(first.highest_price, dec!(138.00));
    assert_eq!(first.lowest_price, dec!(136.05));
    assert_eq!(first.closing_price, dec!(137.90));
    assert_eq!(first.change, dec!(1.85));
    // 千分位逗號必須去掉，否則成交股數會解析失敗而變成 0。
    assert_eq!(first.trading_volume, dec!(7376244));
    assert_eq!(first.trade_value, dec!(1011342122));
    assert_eq!(first.transaction, dec!(5273));
}

#[test]
fn change_range_is_derived_from_the_previous_close() {
    let quotes = parse_stock_day_response(&fixture(), "0050");
    let first = &quotes[0];

    // 前一日收盤 = 137.90 − 1.85 = 136.05，漲幅約 1.36%。
    let expected = (dec!(1.85) / dec!(136.05)) * Decimal::ONE_HUNDRED;
    assert_eq!(first.change_range, expected);
}

#[test]
fn negative_change_is_parsed_with_its_sign() {
    let quotes = parse_stock_day_response(&fixture(), "0050");
    let down_day = quotes
        .iter()
        .find(|quote| quote.date == NaiveDate::from_ymd_opt(2021, 8, 5).expect("日期"))
        .expect("2021-08-05 應在結果中");

    assert_eq!(down_day.change, dec!(-0.25));
    assert!(down_day.change_range < Decimal::ZERO);
}

#[test]
fn non_ok_status_is_treated_as_no_data() {
    let response = StockDayResponse {
        stat: Some("很抱歉，沒有符合條件的資料!".to_owned()),
        title: None,
        fields: None,
        data: None,
    };
    assert!(parse_stock_day_response(&response, "0050").is_empty());
}

#[test]
fn malformed_rows_are_skipped_without_dropping_the_month() {
    let response = StockDayResponse {
        stat: Some("OK".to_owned()),
        title: None,
        fields: None,
        data: Some(vec![
            // 欄位不足。
            vec!["110/08/02".to_owned(), "1".to_owned()],
            // 日期格式錯誤。
            vec![
                "2021-08-03".to_owned(),
                "1".to_owned(),
                "1".to_owned(),
                "10".to_owned(),
                "10".to_owned(),
                "10".to_owned(),
                "10".to_owned(),
                "0".to_owned(),
                "1".to_owned(),
            ],
            // 正常列。
            vec![
                "110/08/04".to_owned(),
                "1,000".to_owned(),
                "10,000".to_owned(),
                "10.00".to_owned(),
                "10.50".to_owned(),
                "9.90".to_owned(),
                "10.20".to_owned(),
                "+0.20".to_owned(),
                "3".to_owned(),
            ],
        ]),
    };

    let quotes = parse_stock_day_response(&response, "0050");
    assert_eq!(quotes.len(), 1);
    assert_eq!(quotes[0].closing_price, dec!(10.20));
}

/// `data` 為 null（TWSE 偶爾如此回應）時視為查無資料，不可 panic。
#[test]
fn missing_data_array_is_treated_as_no_data() {
    let response = StockDayResponse {
        stat: Some("OK".to_owned()),
        title: None,
        fields: None,
        data: None,
    };
    assert!(parse_stock_day_response(&response, "0050").is_empty());

    // stat 缺漏同樣視為查無資料。
    let response = StockDayResponse {
        stat: None,
        title: None,
        fields: None,
        data: Some(vec![]),
    };
    assert!(parse_stock_day_response(&response, "0050").is_empty());
}

/// 收盤價為 `--` 或 0 的列必須整列跳過：0 元收盤會污染後續所有計算。
#[test]
fn rows_without_a_positive_close_are_skipped() {
    let row = |close: &str| {
        vec![
            "110/08/04".to_owned(),
            "1,000".to_owned(),
            "10,000".to_owned(),
            "10.00".to_owned(),
            "10.50".to_owned(),
            "9.90".to_owned(),
            close.to_owned(),
            "+0.20".to_owned(),
            "3".to_owned(),
        ]
    };
    let response = StockDayResponse {
        stat: Some("OK".to_owned()),
        title: None,
        fields: None,
        data: Some(vec![row("--"), row("0.00"), row("-1.00")]),
    };

    assert!(parse_stock_day_response(&response, "0050").is_empty());
}

/// 開高低價缺漏時回退為收盤價，其餘量值欄位回退為 0。
#[test]
fn missing_optional_columns_fall_back_to_safe_values() {
    let response = StockDayResponse {
        stat: Some("OK".to_owned()),
        title: None,
        fields: None,
        data: Some(vec![vec![
            "110/08/04".to_owned(),
            "--".to_owned(),
            "--".to_owned(),
            "--".to_owned(),
            "--".to_owned(),
            "--".to_owned(),
            "10.20".to_owned(),
            "--".to_owned(),
            "--".to_owned(),
        ]]),
    };

    let quotes = parse_stock_day_response(&response, "0050");
    assert_eq!(quotes.len(), 1);
    let quote = &quotes[0];
    assert_eq!(quote.opening_price, dec!(10.20));
    assert_eq!(quote.highest_price, dec!(10.20));
    assert_eq!(quote.lowest_price, dec!(10.20));
    assert_eq!(quote.trading_volume, Decimal::ZERO);
    assert_eq!(quote.trade_value, Decimal::ZERO);
    assert_eq!(quote.transaction, Decimal::ZERO);
    assert_eq!(quote.change, Decimal::ZERO);
    // 漲跌為 0 時前一日收盤等於本日收盤，漲跌幅為 0。
    assert_eq!(quote.change_range, Decimal::ZERO);
}

/// 本益比、股價淨值比與最佳買賣揭示不在此來源，必須維持 0 而非亂填。
#[test]
fn fields_absent_from_this_source_stay_zero() {
    let quotes = parse_stock_day_response(&fixture(), "0050");
    let quote = &quotes[0];

    assert_eq!(quote.price_earning_ratio, Decimal::ZERO);
    assert_eq!(quote.price_to_book_ratio, Decimal::ZERO);
    assert_eq!(quote.last_best_bid_price, Decimal::ZERO);
    assert_eq!(quote.last_best_ask_price, Decimal::ZERO);
}

/// 千分位、正負號與空白都要被吃掉；`--` 與空字串則視為解析失敗。
#[test]
fn number_strips_formatting_and_rejects_placeholders() {
    assert_eq!(number("1,011,342,122").expect("千分位"), dec!(1011342122));
    assert_eq!(number("+0.20").expect("正號"), dec!(0.20));
    assert_eq!(number("-0.25").expect("負號"), dec!(-0.25));
    for raw in ["--", "", "   ", "N/A"] {
        assert!(number(raw).is_err(), "raw={raw:?} 應解析失敗");
    }
}

#[test]
fn parse_roc_date_converts_from_minguo_calendar() {
    assert_eq!(
        parse_roc_date("110/08/02"),
        NaiveDate::from_ymd_opt(2021, 8, 2)
    );
    assert_eq!(
        parse_roc_date(" 99/01/04 "),
        NaiveDate::from_ymd_opt(2010, 1, 4)
    );
    assert_eq!(parse_roc_date("2021/08/02"), None);
    assert_eq!(parse_roc_date("110/13/02"), None);
    assert_eq!(parse_roc_date("--"), None);
}
