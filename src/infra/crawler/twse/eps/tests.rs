//! 公開資訊觀測站季 EPS 爬蟲的測試；連外的測試標 #[ignore]。

use crate::infra::cache::SHARE;

use super::*;
use rust_decimal_macros::dec;

/// 以貼近 MOPS 真實回應形狀的 fixture 驗證整頁解析流程。
///
/// `is_known_symbol` 用固定清單注入，不依賴 SHARE 全域快取——
/// 這正是把它做成閉包參數的目的：解析測試完全離線、無外部狀態。
#[test]
fn parse_eps_html_parses_fixture_rows() {
    // include_str! 的路徑相對於本檔案（twse/eps/tests.rs）→ twse/testdata/。
    const FIXTURE: &str = include_str!("../testdata/eps_t163sb19.html");
    let known = ["2330", "2317"];

    let result = parse_eps_html(FIXTURE, 2025, Quarter::Q3, |symbol| known.contains(&symbol));

    // 有效列只有 2330 與 2317：表頭、備註、空代號與非追蹤代號（9998）都被略過。
    assert_eq!(result.len(), 2);

    assert_eq!(result[0].stock_symbol, "2330");
    assert_eq!(result[0].year, 2025);
    assert_eq!(result[0].quarter, Quarter::Q3);
    assert_eq!(result[0].earnings_per_share, dec!(15.36));

    // EPS 欄為「N/A」時以 0 落地，而不是讓整頁解析失敗。
    assert_eq!(result[1].stock_symbol, "2317");
    assert_eq!(result[1].earnings_per_share, Decimal::ZERO);
}

/// 與目標結構無關的頁面（維護頁、錯誤頁）應回傳空清單，不 panic。
#[test]
fn parse_eps_html_returns_empty_for_unrelated_page() {
    let result = parse_eps_html(
        "<html><body><p>查無資料</p></body></html>",
        2025,
        Quarter::Q3,
        |_| true,
    );
    assert!(result.is_empty());
}

#[tokio::test]
#[ignore = "live test：連線真實外部網站，需要時手動執行"]
async fn test_visit() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 visit");

    match visit(StockExchangeMarket::Listed, 2025, Quarter::Q4).await {
        Ok(list) => {
            dbg!(&list);
            tracing::debug!("list:{:#?}", list);
        }
        Err(why) => {
            tracing::debug!("Failed to visit because: {:?}", why);
        }
    }

    tracing::debug!("結束 visit");
}
