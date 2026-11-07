//! 年度獲利（年度財報）共用型別與解析的測試；連外的測試標 #[ignore]。

use rust_decimal_macros::dec;

use super::*;

// === 年度獲利 HTML 解析（fixture）測試 ===
//
// 這組測試示範本專案的「crawler fixture 測試模式」：
// 1. 把站方頁面的代表性內容存成 `testdata/` 下的檔案（fixture）。
// 2. 用 `include_str!` 在「編譯期」把檔案內容嵌入測試——執行時
//    不需要讀檔、更不需要網路，任何環境（含 CI）都能穩定執行。
// 3. 對「純解析函式」做斷言，涵蓋正常列與各種異常列。
// live 測試（真的連站方）仍保留 #[ignore]，只在懷疑站方改版時手動執行。

/// 驗證 fixture 頁面中的正常資料列被完整解析，異常列被正確略過。
#[test]
fn parse_annual_profits_html_parses_fixture_rows() {
    // include_str! 的路徑是「相對於本原始碼檔案」（本檔位於
    // src/infra/crawler/share/，fixture 仍放在 src/infra/crawler/testdata/，
    // 故需以 ../ 回到上一層），編譯期就會檢查檔案存在，路徑錯會直接編譯失敗。
    const FIXTURE: &str = include_str!("../../testdata/annual_profit.html");

    let profits = parse_annual_profits_html(FIXTURE, "2330").unwrap();

    // fixture 內有 5 個資料列候選：2 筆正常、3 筆異常（欄位不足的備註列、
    // 年度非數字的合計列、EPS 為 N/A 的列）——只有正常的 2 筆會被保留。
    assert_eq!(profits.len(), 2, "應恰好解析出 2 筆正常列: {profits:#?}");

    // 第一筆：民國 112 年 → 西元 2023 年，各欄位對應正確的 <td> 位置。
    assert_eq!(profits[0].stock_symbol, "2330");
    assert_eq!(profits[0].year, 2023);
    assert_eq!(profits[0].sales_per_share, dec!(83.36));
    assert_eq!(profits[0].profit_before_tax, dec!(37.79));
    assert_eq!(profits[0].earnings_per_share, dec!(32.34));

    // 第二筆：民國 111 年 → 西元 2022 年。
    assert_eq!(profits[1].year, 2022);
    assert_eq!(profits[1].earnings_per_share, dec!(39.20));
}

/// 驗證空頁面／完全不含目標表格的頁面回傳空清單而不是錯誤。
///
/// 站方改版把表格 id 換掉時就會出現這種情況——解析不崩潰、
/// 回傳空清單，由上層的資料量檢查（或 live 測試）發現異常。
#[test]
fn parse_annual_profits_html_returns_empty_for_unrelated_page() {
    let profits = parse_annual_profits_html("<html><body>改版了</body></html>", "2330").unwrap();
    assert!(profits.is_empty());
}
