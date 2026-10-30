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
