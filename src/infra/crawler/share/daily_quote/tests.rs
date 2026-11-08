//! 共用日線報價解析的單元測試。

use std::collections::HashMap;

use chrono::NaiveDate;
use rust_decimal_macros::dec;

use super::*;
use crate::core::declare::StockExchange;

/// 建立 TWSE 欄位名稱對照表與一筆有效資料列，供 from_with_map 測試共用。
fn twse_map_and_row() -> (HashMap<&'static str, usize>, Vec<String>) {
    let map = HashMap::from([
        ("證券代號", 0),
        ("成交股數", 1),
        ("成交筆數", 2),
        ("成交金額", 3),
        ("開盤價", 4),
        ("最高價", 5),
        ("最低價", 6),
        ("收盤價", 7),
        ("漲跌(+/-)", 8),
        ("漲跌價差", 9),
        ("最後揭示買價", 10),
        ("最後揭示買量", 11),
        ("最後揭示賣價", 12),
        ("最後揭示賣量", 13),
        ("本益比", 14),
    ]);
    let row = vec![
        "2330".to_string(),
        "1,234,000".to_string(),
        "5,678".to_string(),
        "987,654,321".to_string(),
        "950.5".to_string(),
        "960.5".to_string(),
        "945.5".to_string(),
        "955.5".to_string(),
        "綠".to_string(),
        "12.5".to_string(),
        "955.0".to_string(),
        "100".to_string(),
        "956.0".to_string(),
        "200".to_string(),
        "20.5".to_string(),
    ];
    (map, row)
}

/// 驗證 from_with_map 正常解析（含千分位、綠字轉負值）。
#[test]
fn from_with_map_parses_valid_row() {
    let (map, row) = twse_map_and_row();
    let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();

    let dto = DailyQuoteDto::from_with_map(&row, &map, date).unwrap();

    assert_eq!(dto.symbol, "2330");
    assert_eq!(dto.trading_volume, dec!(1234000));
    assert_eq!(dto.closing_price, dec!(955.5));
    // 「綠」代表下跌，漲跌價差應轉為負值。
    assert_eq!(dto.change, dec!(-12.5));
    assert_eq!(dto.price_earning_ratio, dec!(20.5));
}

/// 驗證欄位改名（對照表缺欄位）時整列被拒絕，而不是補 0。
#[test]
fn from_with_map_rejects_missing_column() {
    let (mut map, row) = twse_map_and_row();
    // 模擬來源把「收盤價」改名：對照表中不再有這個欄位。
    map.remove("收盤價");
    let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();

    let err = DailyQuoteDto::from_with_map(&row, &map, date).unwrap_err();

    assert!(matches!(
        err,
        QuoteParseError::MissingField { field: "收盤價" }
    ));
}

/// 驗證價格欄位含垃圾內容時整列被拒絕。
#[test]
fn from_with_map_rejects_garbage_price() {
    let (map, mut row) = twse_map_and_row();
    row[7] = "corrupted".to_string(); // 收盤價
    let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();

    let err = DailyQuoteDto::from_with_map(&row, &map, date).unwrap_err();

    assert!(matches!(err, QuoteParseError::InvalidDecimal { .. }));
}

/// 驗證漲跌價差是「軟性」欄位：非數字標記補 0，不拒絕整列。
#[test]
fn from_with_map_soft_change_falls_back_to_zero() {
    let (map, mut row) = twse_map_and_row();
    row[9] = "除息".to_string(); // 漲跌價差
    let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();

    let dto = DailyQuoteDto::from_with_map(&row, &map, date).unwrap();

    assert_eq!(dto.change, Decimal::ZERO);
    // 其餘欄位仍完整保留。
    assert_eq!(dto.closing_price, dec!(955.5));
}

/// 驗證 TPEx 位置式解析：正常列成功、含自帶正負號的漲跌。
#[test]
fn from_with_exchange_tpex_parses_valid_row() {
    let row = vec![
        "5483".to_string(),      // 0: 代號
        "中美晶".to_string(),    // 1: 名稱
        "100.00".to_string(),    // 2: 收盤價
        "-1.50".to_string(),     // 3: 漲跌（自帶負號）
        "98.50".to_string(),     // 4: 開盤價
        "101.00".to_string(),    // 5: 最高價
        "98.00".to_string(),     // 6: 最低價
        "10,000".to_string(),    // 7: 成交股數
        "1,000,000".to_string(), // 8: 成交金額
        "500".to_string(),       // 9: 成交筆數
        "100.00".to_string(),    // 10: 最後買價
        "10".to_string(),        // 11: 最後買量
        "100.50".to_string(),    // 12: 最後賣價
        "20".to_string(),        // 13: 最後賣量
    ];
    let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();

    let dto = DailyQuoteDto::from_with_exchange(StockExchange::TPEx, &row, date).unwrap();

    assert_eq!(dto.symbol, "5483");
    assert_eq!(dto.closing_price, dec!(100.00));
    assert_eq!(dto.change, dec!(-1.50));
    assert_eq!(dto.trading_volume, dec!(10000));
}

/// 驗證 TPEx 資料列長度不足（缺欄位）時整列被拒絕。
#[test]
fn from_with_exchange_tpex_rejects_short_row() {
    let row = vec!["5483".to_string(), "中美晶".to_string()];
    let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();

    let err = DailyQuoteDto::from_with_exchange(StockExchange::TPEx, &row, date).unwrap_err();

    assert!(matches!(err, QuoteParseError::MissingField { .. }));
}

/// 一筆有效的 TWSE 位置式資料列（16 欄）；`sign` 為第 9 欄的漲跌方向 HTML。
fn twse_row(sign: &str, change: &str) -> Vec<String> {
    [
        "2330",           // 0: 代號
        "台積電",         // 1: 名稱
        "25,000,000",     // 2: 成交股數
        "30,000",         // 3: 成交筆數
        "25,000,000,000", // 4: 成交金額
        "1,000.00",       // 5: 開盤價
        "1,010.00",       // 6: 最高價
        "995.00",         // 7: 最低價
        "1,005.00",       // 8: 收盤價
        sign,             // 9: 漲跌(+/-)
        change,           // 10: 漲跌價差
        "1,000.00",       // 11: 最後揭示買價
        "100",            // 12: 最後揭示買量
        "1,005.00",       // 13: 最後揭示賣價
        "200",            // 14: 最後揭示賣量
        "22.50",          // 15: 本益比
    ]
    .map(String::from)
    .to_vec()
}

/// TWSE 位置式解析：漲跌方向放在第 9 欄，含 `-` 時把無號的漲跌價差轉負。
#[test]
fn from_with_exchange_twse_applies_sign_column() {
    let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();

    let down = DailyQuoteDto::from_with_exchange(
        StockExchange::TWSE,
        &twse_row("<p style= color:green>-</p>", "5.00"),
        date,
    )
    .unwrap();
    let up = DailyQuoteDto::from_with_exchange(
        StockExchange::TWSE,
        &twse_row("<p style= color:red>+</p>", "5.00"),
        date,
    )
    .unwrap();

    assert_eq!(down.symbol, "2330");
    assert_eq!(down.trading_volume, dec!(25000000));
    assert_eq!(down.closing_price, dec!(1005.00));
    assert_eq!(down.price_earning_ratio, dec!(22.50));
    assert_eq!(down.change, dec!(-5.00));
    assert_eq!(up.change, dec!(5.00));
}

/// 除權息日漲跌價差可能是非數字標記：補 0，不拒絕整列。
#[test]
fn from_with_exchange_twse_soft_change_falls_back_to_zero() {
    let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();

    let dto = DailyQuoteDto::from_with_exchange(StockExchange::TWSE, &twse_row("", "除息"), date)
        .unwrap();

    assert_eq!(dto.change, dec!(0));
    assert_eq!(dto.closing_price, dec!(1005.00));
}

/// TWSE 資料列缺欄或價格欄位是垃圾字串時整列被拒絕，錯誤指出欄位名稱。
#[test]
fn from_with_exchange_twse_rejects_short_row_and_garbage() {
    let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();
    let mut short = twse_row("+", "1.00");
    short.truncate(15);
    let mut garbage = twse_row("+", "1.00");
    garbage[8] = "abc".to_string();

    let missing = DailyQuoteDto::from_with_exchange(StockExchange::TWSE, &short, date).unwrap_err();
    let invalid =
        DailyQuoteDto::from_with_exchange(StockExchange::TWSE, &garbage, date).unwrap_err();

    assert!(
        matches!(missing, QuoteParseError::MissingField { field: "本益比" }),
        "{missing}"
    );
    assert!(
        matches!(
            invalid,
            QuoteParseError::InvalidDecimal {
                field: "收盤價",
                ..
            }
        ),
        "{invalid}"
    );
}

/// 沒有對應解析規則的交易所只留下代號與日期，其餘欄位維持預設值。
#[test]
fn from_with_exchange_other_exchange_keeps_defaults() {
    let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();
    let row = vec!["2330".to_string()];

    let dto = DailyQuoteDto::from_with_exchange(StockExchange::None, &row, date).unwrap();

    assert_eq!(dto.symbol, "2330");
    assert_eq!(dto.closing_price, dec!(0));
}

/// 整列為空時連代號都沒有，視為缺欄位。
#[test]
fn from_with_exchange_rejects_empty_row() {
    let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();

    let err = DailyQuoteDto::from_with_exchange(StockExchange::TWSE, &[], date).unwrap_err();

    assert!(matches!(
        err,
        QuoteParseError::MissingField {
            field: "證券代號"
        }
    ));
}
