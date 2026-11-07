//! quote的測試；連外的測試標 #[ignore]。

use chrono::{Local, TimeDelta, Timelike};
use std::time::Duration;

use crate::infra::cache::SHARE;

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::*;

#[tokio::test]
async fn test_parse_quote_response() {
    let table = Table {
        data: Some(vec![vec![
            "5483".to_string(),      // 0: 代號
            "中美晶".to_string(),    // 1: 名稱
            "100.00".to_string(),    // 2: 收盤價
            "1.50".to_string(),      // 3: 漲跌
            "98.50".to_string(),     // 4: 開盤價
            "101.00".to_string(),    // 5: 最高價
            "98.00".to_string(),     // 6: 最低價
            "10,000".to_string(),    // 7: 成交股數 (含逗號)
            "1,000,000".to_string(), // 8: 成交金額 (含逗號)
            "500".to_string(),       // 9: 成交筆數
            "100.00".to_string(),    // 10: 最後買價
            "10".to_string(),        // 11: 最後買量
            "100.50".to_string(),    // 12: 最後賣價
            "20".to_string(),        // 13: 最後賣量
        ]]),
    };

    let response = QuoteResponse {
        tables: vec![table],
    };

    let pe_ratio = vec![PeRatioAnalysisResponse {
        security_code: "5483".to_string(),
        price_earning_ratio: "12.34".to_string(),
        date: "1150613".to_string(),
    }];

    let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();
    let result = parse_quote_response(response, pe_ratio, date)
        .await
        .unwrap();

    assert_eq!(result.len(), 1);
    let quote = &result[0];
    assert_eq!(quote.symbol, "5483");
    assert_eq!(quote.opening_price, dec!(98.50));
    assert_eq!(quote.highest_price, dec!(101.00));
    assert_eq!(quote.lowest_price, dec!(98.00));
    assert_eq!(quote.closing_price, dec!(100.00));
    assert_eq!(quote.change, dec!(1.50));
    assert_eq!(quote.trading_volume, dec!(10000));
    assert_eq!(quote.trade_value, dec!(1000000));
    assert_eq!(quote.transaction, dec!(500));
    assert_eq!(quote.price_earning_ratio, dec!(12.34));
}

/// 驗證本益比為 `N/A` 這類無資料佔位符時，規則化為 0 且不影響整列。
#[tokio::test]
async fn test_parse_quote_response_pe_ratio_placeholder_becomes_zero() {
    let table = Table {
        data: Some(vec![vec![
            "5483".to_string(),
            "中美晶".to_string(),
            "100.00".to_string(),
            "1.50".to_string(),
            "98.50".to_string(),
            "101.00".to_string(),
            "98.00".to_string(),
            "10,000".to_string(),
            "1,000,000".to_string(),
            "500".to_string(),
            "100.00".to_string(),
            "10".to_string(),
            "100.50".to_string(),
            "20".to_string(),
        ]]),
    };
    let response = QuoteResponse {
        tables: vec![table],
    };
    // 本益比 API 回傳無資料佔位符。
    let pe_ratio = vec![PeRatioAnalysisResponse {
        security_code: "5483".to_string(),
        price_earning_ratio: "N/A".to_string(),
        date: "1150612".to_string(),
    }];

    let date = NaiveDate::from_ymd_opt(2026, 6, 12).unwrap();
    let result = parse_quote_response(response, pe_ratio, date)
        .await
        .unwrap();

    assert_eq!(result.len(), 1);
    // 佔位符規則化為 0；行情主體欄位完整保留。
    assert_eq!(result[0].price_earning_ratio, Decimal::ZERO);
    assert_eq!(result[0].closing_price, dec!(100.00));
}

/// 本益比資料日期不是當天（openapi 還停在前一個交易日）時不採用，本益比留 0。
#[tokio::test]
async fn test_parse_quote_response_ignores_stale_pe_ratio() {
    let row: Vec<String> = [
        "5483",
        "中美晶",
        "100.00",
        "1.50",
        "98.50",
        "101.00",
        "98.00",
        "10,000",
        "1,000,000",
        "500",
        "100.00",
        "10",
        "100.50",
        "20",
    ]
    .iter()
    .map(|cell| cell.to_string())
    .collect();
    let response = QuoteResponse {
        tables: vec![Table {
            data: Some(vec![row]),
        }],
    };
    let pe_ratio = vec![PeRatioAnalysisResponse {
        security_code: "5483".to_string(),
        price_earning_ratio: "12.34".to_string(),
        date: "1150611".to_string(),
    }];

    let date = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
    let result = parse_quote_response(response, pe_ratio, date)
        .await
        .unwrap();

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].price_earning_ratio, Decimal::ZERO);
    assert_eq!(result[0].closing_price, dec!(100.00));
}

/// 驗證欄位毀損的資料列會被拒絕；單列來源全毀時整批失敗。
#[tokio::test]
async fn test_parse_quote_response_rejects_corrupted_row() {
    let table = Table {
        data: Some(vec![vec![
            "5483".to_string(),
            "中美晶".to_string(),
            "corrupted".to_string(), // 收盤價放垃圾內容
            "1.50".to_string(),
            "98.50".to_string(),
            "101.00".to_string(),
            "98.00".to_string(),
            "10,000".to_string(),
            "1,000,000".to_string(),
            "500".to_string(),
            "100.00".to_string(),
            "10".to_string(),
            "100.50".to_string(),
            "20".to_string(),
        ]]),
    };
    let response = QuoteResponse {
        tables: vec![table],
    };

    let date = NaiveDate::from_ymd_opt(2026, 6, 12).unwrap();
    // 僅有的一列被拒絕（100% > 10% 門檻），整批應回傳錯誤。
    let err = parse_quote_response(response, vec![], date)
        .await
        .unwrap_err();

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
    // now -= Duration::days(3);

    tracing::debug!("開始 visit");

    match visit(now.date_naive()).await {
        Err(why) => {
            tracing::debug!("Failed to visit because: {:?}", why);
        }
        Ok(list) => {
            tracing::debug!("data:{:#?}", list);
        }
    }

    tracing::debug!("結束 visit");
    tokio::time::sleep(Duration::from_millis(500)).await;
}
