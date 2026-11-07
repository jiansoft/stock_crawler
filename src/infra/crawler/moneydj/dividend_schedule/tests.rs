//! MoneyDJ 股利政策表（除權除息日程）採集器的測試；連外的測試標 #[ignore]。

use rust_decimal_macros::dec;

use super::*;

/// 以真實頁面片段 fixture 驗證解析結果。
///
/// 涵蓋：除權息（同時有股票與現金）、純除息、ETF 的 `AP` 前綴代號、
/// 純除權列必須忽略不相干的現金股利發放日。
#[test]
fn test_parse_schedules_with_fixture() {
    // include_str! 的路徑相對於本檔案（moneydj/dividend_schedule/tests.rs）→ moneydj/testdata/。
    const FIXTURE: &str = include_str!("../testdata/dividend_schedule_zey.html");
    let result = parse_schedules(FIXTURE);

    assert_eq!(result.len(), 4);

    // 除權息：股票與現金股利都要取到，發放日 10/08 與除權息日同年。
    let both = &result[0];
    assert_eq!(both.stock_symbol, "2442");
    assert_eq!(both.name, "新美齊");
    assert_eq!(both.ex_date, NaiveDate::from_ymd_opt(2026, 9, 8).unwrap());
    assert!(both.is_cash);
    assert!(both.is_stock);
    assert_eq!(both.cash_dividend, Some(dec!(2.03186184)));
    assert_eq!(both.stock_dividend, Some(dec!(0.711151634)));
    assert_eq!(both.cash_payable_date, NaiveDate::from_ymd_opt(2026, 10, 8));

    // ETF 的連結前綴是 AP，代號一樣要正確取出。
    let etf = result
        .iter()
        .find(|item| item.stock_symbol == "00400A")
        .expect("00400A should exist");
    assert!(etf.is_cash);
    assert!(!etf.is_stock);
    assert_eq!(etf.cash_dividend, Some(dec!(0.12)));
    assert_eq!(etf.stock_dividend, None);

    // 純除權列：頁面上填的發放日屬於另一次除息，必須忽略。
    let stock_only = result
        .iter()
        .find(|item| !item.is_cash && item.is_stock)
        .expect("stock-only row should exist");
    assert_eq!(stock_only.stock_symbol, "4549");
    assert_eq!(stock_only.stock_dividend, Some(dec!(0.2)));
    assert!(stock_only.cash_payable_date.is_none());
}

/// 跨年發放：12 月除息、隔年 1 月發放，年份要加一。
#[test]
fn test_parse_payable_date_rolls_over_year() {
    let ex_date = NaiveDate::from_ymd_opt(2026, 12, 20).unwrap();

    assert_eq!(
        parse_payable_date("01/15", ex_date),
        NaiveDate::from_ymd_opt(2027, 1, 15)
    );
    assert_eq!(
        parse_payable_date("12/25", ex_date),
        NaiveDate::from_ymd_opt(2026, 12, 25)
    );
    assert_eq!(parse_payable_date("", ex_date), None);
    assert_eq!(parse_payable_date("2026/12/25", ex_date), None);
}

#[tokio::test]
#[ignore]
async fn test_visit() {
    dotenvy::dotenv().ok();

    match visit().await {
        Err(why) => println!("取得 MoneyDJ 股利政策表失敗: {:?}", why),
        Ok(result) => {
            println!("股利政策表 {} 筆", result.len());
            for item in result.iter().take(5) {
                println!("{:?}", item);
            }
        }
    }
}
