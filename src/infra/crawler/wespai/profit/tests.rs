//! profit的測試；連外的測試標 #[ignore]。

use std::time::Duration;

use super::*;
use rust_decimal_macros::dec;

/// 以貼近真實頁面形狀的 fixture 驗證整頁解析流程。
#[test]
fn parse_profit_html_parses_fixture_rows() {
    // include_str! 的路徑相對於本檔案（wespai/profit/tests.rs）→ wespai/testdata/。
    const FIXTURE: &str = include_str!("../testdata/profit.html");

    let profits = parse_profit_html(FIXTURE).unwrap();

    assert_eq!(profits.len(), 2);

    // 年度來自標題「2024年報獲利能力」中的四位數。
    let tsmc = &profits[0];
    assert_eq!(tsmc.year, 2024);
    assert_eq!(tsmc.security_code, "2330");
    assert_eq!(tsmc.gross_profit, dec!(56.12));
    assert_eq!(tsmc.operating_profit_margin, dec!(45.68));
    assert_eq!(tsmc.pre_tax_income, dec!(48.63));
    assert_eq!(tsmc.net_income, dec!(43.06));
    assert_eq!(tsmc.net_asset_value_per_share, dec!(155.86));
    assert_eq!(tsmc.sales_per_share, dec!(104.88));
    assert_eq!(tsmc.profit_before_tax, dec!(51.01));
    assert_eq!(tsmc.return_on_equity, dec!(30.24));
    assert_eq!(tsmc.return_on_assets, dec!(19.35));
    assert_eq!(tsmc.earnings_per_share, dec!(45.25));

    // 金融股的毛利率欄是「-」→ 以 0 落地，不影響其他欄位。
    let esun = &profits[1];
    assert_eq!(esun.security_code, "2884");
    assert_eq!(esun.gross_profit, Decimal::ZERO);
    assert_eq!(esun.earnings_per_share, dec!(1.35));
}

/// 頁面標題缺少年度（改版或載到錯誤頁）時必須整頁報錯——
/// 年度是資料的 key，掛錯年度比沒抓到更糟。
#[test]
fn parse_profit_html_rejects_page_without_year() {
    let error = parse_profit_html("<html><body><p>維護中</p></body></html>").unwrap_err();
    assert!(error.to_string().contains("Failed to select"));
}

#[tokio::test]
#[ignore]
async fn test_visit() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 visit");

    match visit().await {
        Ok(e) => {
            tracing::debug!("{:#?}", e);
        }
        Err(why) => {
            tracing::debug!("Failed to visit because {:?}", why);
        }
    }

    tracing::debug!("結束 visit");
    tokio::time::sleep(Duration::from_secs(5)).await;
}
