//! 持股季報通知查詢的整合測試。

use super::*;

/// 季報通知的回歸測試：`financial_statement.year` 是 `bigint`，
/// `HoldingFinancialAlert.year` 是 `i32`，少了 SQL 的 CAST 就會在解碼時整個查詢失敗。
///
/// 這個 bug 在正式站存在過，症狀是每季 04:00 的持股季報通知靜靜地不送、
/// 只在 error log 留下 `mismatched types ... INT4 ... INT8`。
/// 斷言的重點是「查詢不回錯」，而不是回了幾筆。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_holding_financial_alerts_decodes_bigint_year() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_fetch_holding_financial_alerts_decodes_bigint_year：無資料庫連接");
        return;
    }

    cleanup().await;
    seed().await;

    let alerts = fetch_holding_financial_alerts(2026, "Q2")
        .await
        .expect("year 是 bigint，沒有 CAST 會在這裡炸掉");

    let a = alerts
        .iter()
        .find(|alert| alert.stock_symbol == FAKE_SYMBOL_A)
        .expect("測試甲的 2026 Q2 財報應該被查到");
    assert_eq!(a.year, 2026);
    assert_eq!(a.quarter, "Q2");
    assert_eq!(a.earnings_per_share, dec!(1.8));

    cleanup().await;
}
