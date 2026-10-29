//! 上市每日收盤行情（MI_INDEX）解析與抓取的測試。

use chrono::{TimeDelta, Timelike};
use std::time::Duration;

use crate::infra::cache::SHARE;

use rust_decimal_macros::dec;

use super::*;

#[test]
fn test_is_twse_quote_table() {
    let table = Table {
        title: Some("每日收盤行情(全部(不含權證、牛熊證))".to_string()),
        fields: Some(vec![
            "證券代號".to_string(),
            "證券名稱".to_string(),
            "成交股數".to_string(),
            "成交筆數".to_string(),
            "成交金額".to_string(),
            "開盤價".to_string(),
            "最高價".to_string(),
            "最低價".to_string(),
            "收盤價".to_string(),
            "漲跌(+/-)".to_string(),
            "漲跌價差".to_string(),
            "最後揭示買價".to_string(),
            "最後揭示買量".to_string(),
            "最後揭示賣價".to_string(),
            "最後揭示賣量".to_string(),
            "本益比".to_string(),
        ]),
        data: None,
        hints: None,
    };

    assert!(is_twse_quote_table(&table));
}

#[tokio::test]
async fn test_parse_listed_response() {
    let table = Table {
        title: Some("每日收盤行情(全部(不含權證、牛熊證))".to_string()),
        fields: Some(vec![
            "證券代號".to_string(),
            "證券名稱".to_string(),
            "成交股數".to_string(),
            "成交筆數".to_string(),
            "成交金額".to_string(),
            "開盤價".to_string(),
            "最高價".to_string(),
            "最低價".to_string(),
            "收盤價".to_string(),
            "漲跌(+/-)".to_string(),
            "漲跌價差".to_string(),
            "最後揭示買價".to_string(),
            "最後揭示買量".to_string(),
            "最後揭示賣價".to_string(),
            "最後揭示賣量".to_string(),
            "本益比".to_string(),
        ]),
        data: Some(vec![vec![
            "2330".to_string(),      // 證券代號
            "台積電".to_string(),    // 證券名稱
            "10,000".to_string(),    // 成交股數 (含逗號)
            "100".to_string(),       // 成交筆數
            "5,000,000".to_string(), // 成交金額 (含逗號)
            "500.00".to_string(),    // 開盤價
            "505.00".to_string(),    // 最高價
            "499.00".to_string(),    // 最低價
            "502.00".to_string(),    // 收盤價
            "+".to_string(),         // 漲跌(+/-)
            "2.00".to_string(),      // 漲跌價差
            "502.00".to_string(),
            "10".to_string(),
            "503.00".to_string(),
            "20".to_string(),
            "15.5".to_string(),
        ]]),
        hints: None,
    };

    let response = ListedResponse {
        stat: Some("OK".to_string()),
        tables: vec![table],
    };

    let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();
    let result = parse_listed_response(response, date).await.unwrap();

    assert_eq!(result.len(), 1);
    let quote = &result[0];
    assert_eq!(quote.symbol, "2330");
    assert_eq!(quote.opening_price, dec!(500.00));
    assert_eq!(quote.highest_price, dec!(505.00));
    assert_eq!(quote.lowest_price, dec!(499.00));
    assert_eq!(quote.closing_price, dec!(502.00));
    assert_eq!(quote.change, dec!(2.00));
    assert_eq!(quote.trading_volume, dec!(10000));
    assert_eq!(quote.trade_value, dec!(5000000));
    assert_eq!(quote.transaction, dec!(100));
    assert_eq!(quote.price_earning_ratio, dec!(15.5));
}

/// TWSE 在部分日期會於指數表格送出 `null` 儲存格，整份回應仍必須解析成功。
///
/// 這個情境曾讓 2019-06 與 2019-08 共 14 個交易日的上市報價回補全數失敗：
/// 報價表格本身完全正常，卻因為另一張指數表格裡的 `null` 而整批解析不出來。
#[tokio::test]
async fn test_parse_listed_response_with_null_cells() {
    let raw = include_str!("../testdata/mi_index_null_cells_20190610.json");
    let response: ListedResponse =
        serde_json::from_str(raw).expect("含 null 儲存格的回應仍應反序列化成功");

    let date = NaiveDate::from_ymd_opt(2019, 6, 10).unwrap();
    let result = parse_listed_response(response, date).await.unwrap();

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].symbol, "0050");
    assert_eq!(result[0].closing_price, dec!(79.05));
}

/// 2026-10-23 臨時停市（不在假日表上）的實際回應：沒有 `tables`，stat 是
/// 「沒有符合條件的資料」。要解析成 0 筆，而不是反序列化失敗或 API 錯誤。
#[tokio::test]
async fn test_parse_listed_response_no_data_without_tables() {
    let raw = include_str!("../testdata/mi_index_no_data_20261023.json");
    let response: ListedResponse =
        serde_json::from_str(raw).expect("沒有 tables 的回應仍應反序列化成功");
    assert!(response.tables.is_empty());

    let date = NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
    let result = parse_listed_response(response, date).await.unwrap();
    assert!(result.is_empty());
}

#[test]
fn test_is_no_data_stat() {
    assert!(is_no_data_stat("很抱歉，沒有符合條件的資料!"));
    assert!(is_no_data_stat("很抱歉，查無資料"));
    assert!(is_no_data_stat("查詢日期大於當前日期，請重新查詢!"));
    assert!(!is_no_data_stat("OK"));
    assert!(!is_no_data_stat("系統忙碌中"));
}

/// 產生一筆指定代號的有效 TWSE 資料列，供批次解析測試使用。
fn make_valid_row(symbol: &str) -> Vec<String> {
    vec![
        symbol.to_string(),
        format!("{symbol}公司"),
        "10,000".to_string(),
        "100".to_string(),
        "5,000,000".to_string(),
        "500.00".to_string(),
        "505.00".to_string(),
        "499.00".to_string(),
        "502.00".to_string(),
        "+".to_string(),
        "2.00".to_string(),
        "502.00".to_string(),
        "10".to_string(),
        "503.00".to_string(),
        "20".to_string(),
        "15.5".to_string(),
    ]
}

/// TWSE 收盤行情表頭欄位清單（與 `make_valid_row` 的欄位順序一致）。
fn quote_table_fields() -> Vec<String> {
    [
        "證券代號",
        "證券名稱",
        "成交股數",
        "成交筆數",
        "成交金額",
        "開盤價",
        "最高價",
        "最低價",
        "收盤價",
        "漲跌(+/-)",
        "漲跌價差",
        "最後揭示買價",
        "最後揭示買量",
        "最後揭示賣價",
        "最後揭示賣量",
        "本益比",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// 驗證少量壞列（低於 10% 門檻）會被跳過，其餘資料仍正常回傳。
#[tokio::test]
async fn test_parse_listed_response_skips_minor_bad_rows() {
    // 19 筆有效 + 1 筆收盤價毀損（5% < 10% 門檻）。
    let mut rows: Vec<Vec<String>> = (0..19)
        .map(|i| make_valid_row(&format!("91{i:02}")))
        .collect();
    let mut bad_row = make_valid_row("9199");
    bad_row[8] = "corrupted".to_string(); // 收盤價放垃圾內容
    rows.push(bad_row);

    let response = ListedResponse {
        stat: Some("OK".to_string()),
        tables: vec![Table {
            title: Some("每日收盤行情".to_string()),
            fields: Some(quote_table_fields()),
            data: Some(rows),
            hints: None,
        }],
    };

    let date = NaiveDate::from_ymd_opt(2026, 6, 12).unwrap();
    let result = parse_listed_response(response, date).await.unwrap();

    // 壞列被拒絕、其餘 19 筆完整保留；沒有任何一筆以零值混入。
    assert_eq!(result.len(), 19);
    assert!(result.iter().all(|dto| dto.symbol != "9199"));
}

/// 驗證壞列比例超過門檻時整批失敗，避免大量缺漏資料悄悄入庫。
#[tokio::test]
async fn test_parse_listed_response_fails_when_too_many_bad_rows() {
    // 2 筆中壞 1 筆（50% > 10% 門檻），應整批回傳錯誤。
    let good_row = make_valid_row("9201");
    let mut bad_row = make_valid_row("9202");
    bad_row[8] = "corrupted".to_string();

    let response = ListedResponse {
        stat: Some("OK".to_string()),
        tables: vec![Table {
            title: Some("每日收盤行情".to_string()),
            fields: Some(quote_table_fields()),
            data: Some(vec![good_row, bad_row]),
            hints: None,
        }],
    };

    let date = NaiveDate::from_ymd_opt(2026, 6, 12).unwrap();
    let err = parse_listed_response(response, date).await.unwrap_err();

    assert!(err.to_string().contains("source format may have changed"));
}

#[tokio::test]
#[ignore]
async fn test_visit() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let mut now = Local::now();
    if now.hour() < 15 {
        now -= TimeDelta::try_days(1).unwrap();
    }
    //now -= Duration::days(3);

    tracing::debug!("開始 visit");

    match visit(now.date_naive()).await {
        Err(why) => {
            tracing::debug!("Failed to visit because: {:?}", why);
        }
        Ok(list) => {
            tracing::debug!("data count: {}, detail:{:#?}", list.len(), list);
        }
    }

    tracing::debug!("結束 visit");
    tokio::time::sleep(Duration::from_millis(500)).await;
}
