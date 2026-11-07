//! HiStock 年度財報採集的測試；連外的測試標 #[ignore]。

use super::*;

#[test]
fn test_parse_annual_profit_from_text_nodes() {
    let texts = vec![
        "其他".to_string(),
        "季別/年度".to_string(),
        "2024".to_string(),
        "2023".to_string(),
        "2022".to_string(),
        "Q4".to_string(),
        "總計".to_string(),
        "1.16".to_string(),
        "3.25".to_string(),
        "--".to_string(),
    ];

    let result = parse_annual_profit_from_text_nodes("2838", &texts).unwrap();

    assert_eq!(result.len(), 3);
    assert_eq!(result[0].year, 2024);
    assert_eq!(
        result[0].earnings_per_share,
        Decimal::from_str_exact("1.16").unwrap()
    );
    assert_eq!(result[1].year, 2023);
    assert_eq!(
        result[1].earnings_per_share,
        Decimal::from_str_exact("3.25").unwrap()
    );
    assert_eq!(result[2].year, 2022);
    assert_eq!(result[2].earnings_per_share, Decimal::ZERO);
}

/// 以貼近真實「每股盈餘」頁面形狀的 fixture 驗證完整解析路徑。
///
/// 上面的 `test_parse_annual_profit_from_text_nodes` 餵的是手工組好的
/// 文字節點陣列；這裡則從 HTML 開始走 [`parse_annual_profit_html`]，
/// 連同前段的「HTML → 文字節點抽取」（trim、濾空白、頁面雜訊不干擾定位）
/// 一併覆蓋。
#[test]
fn parse_annual_profit_html_parses_fixture() {
    // include_str! 的路徑相對於本檔案（histock/annual_profit/tests.rs）→ histock/testdata/。
    const FIXTURE: &str = include_str!("../testdata/annual_profit.html");

    let result = parse_annual_profit_html("2330", FIXTURE).unwrap();

    // 年度欄有三年：2025-（年度未結束、帶結尾連字號）、2024、2023。
    assert_eq!(result.len(), 3);

    // 「2025-」應正規化為 2025；其總計欄為「--」→ EPS 視為 0。
    assert_eq!(result[0].year, 2025);
    assert_eq!(result[0].earnings_per_share, Decimal::ZERO);

    assert_eq!(result[1].year, 2024);
    assert_eq!(
        result[1].earnings_per_share,
        Decimal::from_str_exact("45.25").unwrap()
    );
    assert_eq!(result[2].year, 2023);
    assert_eq!(
        result[2].earnings_per_share,
        Decimal::from_str_exact("32.34").unwrap()
    );

    // 此頁未提供每股營收與稅前淨利，依模組現行策略以 0 回填。
    assert_eq!(result[1].sales_per_share, Decimal::ZERO);
    assert_eq!(result[1].profit_before_tax, Decimal::ZERO);
    assert_eq!(result[1].stock_symbol, "2330");
}

/// 頁面缺少「季別/年度」標頭（例如改版或載到錯誤頁）時必須明確報錯，
/// 讓呼叫端知道來源異常，而不是回傳空集合被誤當「沒有資料」。
#[test]
fn parse_annual_profit_html_rejects_unrelated_page() {
    let error =
        parse_annual_profit_html("2330", "<html><body><h1>404</h1></body></html>").unwrap_err();
    assert!(error.to_string().contains("annual header row"));
}

#[tokio::test]
#[ignore]
async fn test_visit() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 histock::annual_profit::visit");

    match visit("2330").await {
        Ok(result) => {
            dbg!(&result);
            tracing::debug!("histock : {:#?}", result);
        }
        Err(why) => {
            tracing::debug!("Failed to histock::annual_profit::visit because {:?}", why)
        }
    }

    tracing::debug!("結束 histock::annual_profit::visit");
}
