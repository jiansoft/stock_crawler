//! 公開資訊觀測站「上市公司股利分派情形」採集器的測試；連外的測試標 #[ignore]。

use rust_decimal_macros::dec;

use super::*;

/// 以真實回應 fixture 驗證解析結果。
///
/// 涵蓋：年度／上半年／單季三種期別、盈餘配股、資本公積配息、民國年轉西元。
#[test]
fn test_parse_allotments_with_fixture() {
    // include_str! 的路徑相對於本檔案（mops/dividend_allotment/tests.rs）→ mops/testdata/。
    const FIXTURE: &str = include_str!("../testdata/dividend_allotment_t187ap45_L.json");
    let rows: Vec<DividendAllotmentRaw> =
        serde_json::from_str(FIXTURE).expect("fixture should parse");
    let result = parse_allotments(rows);

    assert_eq!(result.len(), 5);

    // 年度配息 + 配股：民國 114 → 西元 2025，季別為空字串。
    let annual = &result[0];
    assert_eq!(annual.stock_symbol, "1231");
    assert_eq!(annual.year_of_dividend, 2025);
    assert_eq!(annual.quarter, "");
    assert_eq!(annual.earnings_cash, dec!(1.5));
    assert_eq!(annual.earnings_stock, dec!(1.0));
    assert_eq!(annual.cash_dividend(), dec!(1.5));
    assert_eq!(annual.stock_dividend(), dec!(1.0));
    assert_eq!(annual.period_start, NaiveDate::from_ymd_opt(2025, 1, 1));
    assert_eq!(annual.period_end, NaiveDate::from_ymd_opt(2025, 12, 31));

    // 資本公積發放的現金要算進公積現金股利，不能記成盈餘配息。
    let reserve = result
        .iter()
        .find(|item| item.stock_symbol == "1101")
        .expect("1101 should exist");
    assert_eq!(reserve.earnings_cash, dec!(0));
    assert_eq!(reserve.capital_reserve_cash, dec!(0.8));
    assert_eq!(reserve.cash_dividend(), dec!(0.8));

    // 上半年配息 → H1。
    let half = result
        .iter()
        .find(|item| item.quarter == "H1")
        .expect("H1 should exist");
    assert_eq!(half.stock_symbol, "1315");
    assert_eq!(half.year_of_dividend, 2026);

    // 單季配息 → Q1／Q4 各一筆。
    assert!(result.iter().any(|item| item.quarter == "Q1"));
    assert!(result.iter().any(|item| item.quarter == "Q4"));
}

#[test]
fn test_resolve_quarter_falls_back_to_period() {
    let start = NaiveDate::from_ymd_opt(2025, 4, 1);
    let end = NaiveDate::from_ymd_opt(2025, 6, 30);

    // 已知文字優先。
    assert_eq!(resolve_quarter("第3季", start, end).as_deref(), Some("Q3"));
    // 未知文字時改用期間推算。
    assert_eq!(resolve_quarter("其他", start, end).as_deref(), Some("Q2"));
    // 期間也不完整時必須是 None，不可退回代表「年度」的空字串。
    assert_eq!(resolve_quarter("其他", None, None), None);
}

#[test]
fn test_parse_period() {
    let (start, end) = parse_period("1140101~1141231");
    assert_eq!(start, NaiveDate::from_ymd_opt(2025, 1, 1));
    assert_eq!(end, NaiveDate::from_ymd_opt(2025, 12, 31));

    let (start, end) = parse_period("");
    assert!(start.is_none());
    assert!(end.is_none());
}

#[tokio::test]
#[ignore]
async fn test_visit() {
    dotenvy::dotenv().ok();

    match visit().await {
        Err(why) => println!("取得上市公司股利分派情形失敗: {:?}", why),
        Ok(result) => {
            println!("股利分派情形 {} 筆", result.len());
            for item in result.iter().take(5) {
                println!("{:?}", item);
            }
        }
    }
}
