//! TWSE 上市除權除息預告表採集器的測試；連外的測試標 #[ignore]。

use rust_decimal_macros::dec;

use super::*;

/// 以真實回應 fixture 驗證解析結果。
///
/// 涵蓋：純除息、除權息、現金股利未公布（ETF 的「待公告」）、
/// 民國日期轉西元、無償配股率換算成元。
#[test]
fn test_parse_announcements_with_fixture() {
    // include_str! 的路徑相對於本檔案（twse/ex_dividend_announcement/tests.rs）→ twse/testdata/。
    const FIXTURE: &str = include_str!("../testdata/ex_dividend_twt48u.json");
    let rows: Vec<Twt48uRaw> = serde_json::from_str(FIXTURE).expect("fixture should parse");
    let result = parse_announcements(rows);

    assert_eq!(result.len(), 5);

    let first = &result[0];
    assert_eq!(first.stock_symbol, "00400A");
    assert_eq!(
        first.ex_date,
        chrono::NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()
    );
    assert!(first.is_cash);
    assert!(!first.is_stock);
    assert_eq!(first.cash_dividend, Some(dec!(0.120000)));
    assert_eq!(first.market, StockExchangeMarket::Listed);

    // ETF 尚未公告實際收益分配金額 → None，而不是 0。
    let unannounced = result
        .iter()
        .find(|item| item.stock_symbol == "00401A")
        .expect("00401A should exist");
    assert!(unannounced.cash_dividend.is_none());

    // 權息同時發放：無償配股率 0.08 → 股票股利 0.8 元。
    let both = result
        .iter()
        .find(|item| item.stock_symbol == "1438")
        .expect("1438 should exist");
    assert!(both.is_cash);
    assert!(both.is_stock);
    assert_eq!(both.stock_dividend_ratio, Some(dec!(0.08000000)));
    assert_eq!(both.stock_dividend(), Some(dec!(0.8000000)));
}

#[test]
fn test_parse_optional_decimal_treats_blank_as_unannounced() {
    assert_eq!(parse_optional_decimal(""), None);
    assert_eq!(parse_optional_decimal("  "), None);
    assert_eq!(parse_optional_decimal("-"), None);
    assert_eq!(parse_optional_decimal("0.000000"), Some(dec!(0)));
    assert_eq!(parse_optional_decimal("1.25"), Some(dec!(1.25)));
}

/// 日期格式異常的資料列必須被丟棄，而不是帶著錯誤日期往下走。
#[test]
fn test_parse_announcements_skips_invalid_date() {
    let rows = vec![Twt48uRaw {
        date: "abc".to_string(),
        code: "2330".to_string(),
        name: "台積電".to_string(),
        ex_dividend: "息".to_string(),
        stock_dividend_ratio: "".to_string(),
        subscription_ratio: "".to_string(),
        cash_dividend: "5.0".to_string(),
    }];

    assert!(parse_announcements(rows).is_empty());
}

/// 純現增除權不是股利事件，必須丟棄；同時有配股的除權息要保留並維持除權。
///
/// 資料取自 2026-09-23 的 `TWT48U_ALL`：2890 永豐金現增、2614 東森配股＋現增。
#[test]
fn test_parse_announcements_drops_pure_rights_issue() {
    let rows: Vec<Twt48uRaw> = serde_json::from_str(
        r#"[
            {"Date":"1151007","Code":"2890","Name":"永豐金","Exdividend":"權",
             "StockDividendRatio":"","SubscriptionRatio":"0.04329540","CashDividend":"0"},
            {"Date":"1151001","Code":"2614","Name":"東森","Exdividend":"權息",
             "StockDividendRatio":"0.08000000","SubscriptionRatio":"0.38195352",
             "CashDividend":"0.400000"}
        ]"#,
    )
    .expect("rows should parse");

    let result = parse_announcements(rows);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].stock_symbol, "2614");
    assert!(result[0].is_cash);
    assert!(result[0].is_stock);
}

#[tokio::test]
#[ignore]
async fn test_visit() {
    dotenvy::dotenv().ok();

    match visit().await {
        Err(why) => println!("取得上市除權息預告表失敗: {:?}", why),
        Ok(result) => {
            println!("上市除權息預告 {} 筆", result.len());
            for item in result.iter().take(5) {
                println!("{:?}", item);
            }
        }
    }
}
